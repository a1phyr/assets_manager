use std::{
    any::{Any, TypeId},
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    entry::{CacheEntry, CacheEntryInner},
    loader::Loader,
    utils::{extension_of, BorrowedKey, HashMap, HashSet, OwnedKey},
    Asset, AssetCache, Compound, SharedString,
};

use super::dependencies::Dependencies;

trait AnyAsset: Any + Send + Sync {
    fn reload(self: Box<Self>, entry: CacheEntryInner);
    fn create(self: Box<Self>, id: SharedString) -> CacheEntry;
}

impl<A: Asset> AnyAsset for A {
    fn reload(self: Box<Self>, entry: CacheEntryInner) {
        entry.handle::<A>().either(
            |_| {
                log::error!(
                    "Static asset registered for hot-reloading: {}",
                    std::any::type_name::<A>()
                )
            },
            |e| e.write(*self),
        );
    }

    fn create(self: Box<Self>, id: SharedString) -> CacheEntry {
        CacheEntry::new::<A>(*self, id)
    }
}

type LoadFn = fn(content: Cow<[u8]>, ext: &str, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>>;

fn load<A: Asset>(
    content: Cow<[u8]>,
    ext: &str,
    id: &str,
    path: &Path,
) -> Option<Box<dyn AnyAsset>> {
    match A::Loader::load(content, ext) {
        Ok(asset) => Some(Box::new(asset)),
        Err(err) => {
            log::warn!(
                "Error reloading \"{}\" from \"{}\": {}",
                id,
                path.display(),
                err
            );
            None
        }
    }
}

pub(crate) type ReloadFn = fn(cache: &AssetCache, id: &str) -> Option<HashSet<OwnedKey>>;

#[allow(clippy::redundant_closure)]
fn reload<T: Compound>(cache: &AssetCache, id: &str) -> Option<HashSet<OwnedKey>> {
    let key = BorrowedKey::new::<T>(id);
    let handle = cache.assets.get_entry(key)?.handle();
    let entry = handle.either(
        |_| {
            log::error!(
                "Static asset registered for hot-reloading: {}",
                std::any::type_name::<T>()
            );
            None
        },
        |e| Some(e),
    )?;

    match cache.record_load::<T>(id) {
        Ok((asset, deps)) => {
            entry.write(asset);
            log::info!("Reloading \"{}\"", id);
            Some(deps)
        }
        Err(err) => {
            log::warn!("Error reloading \"{}\": {}", id, err);
            None
        }
    }
}

/// Invariant: the TypeId is the same as the one of the value returned by the
/// LoadFn.
pub(crate) struct AssetReloadInfos(PathBuf, SharedString, TypeId, LoadFn);

pub(crate) struct CompoundReloadInfos(OwnedKey, HashSet<OwnedKey>, ReloadFn);

/// A update to the list of watched paths
#[non_exhaustive]
pub(crate) enum UpdateMessage {
    Clear,
    AddAsset(AssetReloadInfos),
    AddCompound(CompoundReloadInfos),
}

#[allow(missing_debug_implementations)]
pub struct PublicUpdateMessage(pub(crate) UpdateMessage);

impl PublicUpdateMessage {
    #[inline]
    pub(crate) fn add_asset<A: Asset>(path: PathBuf, id: SharedString) -> Self {
        Self(UpdateMessage::AddAsset(AssetReloadInfos(
            path,
            id,
            TypeId::of::<A>(),
            load::<A>,
        )))
    }

    #[inline]
    pub(crate) fn add_compound<A: Compound>(id: SharedString, deps: HashSet<OwnedKey>) -> Self {
        let key = OwnedKey::new::<A>(id);
        Self(UpdateMessage::AddCompound(CompoundReloadInfos(
            key,
            deps,
            reload::<A>,
        )))
    }

    #[inline]
    pub(crate) fn clear() -> Self {
        Self(UpdateMessage::Clear)
    }
}

/// A map type -> `T`
///
/// We could use a `HashMap` here, but the length of this `Vec` is unlikely to
/// exceed 2 or 3. It would mean that the same file is used to load several
/// assets types, which is uncommon but possible.
struct Types<T>(Vec<(TypeId, T)>);

impl<T> Types<T> {
    #[inline]
    const fn new() -> Self {
        Types(Vec::new())
    }

    fn get(&self, type_id: TypeId) -> Option<&T> {
        for (id, t) in &self.0 {
            if *id == type_id {
                return Some(t);
            }
        }
        None
    }

    #[inline]
    fn insert(&mut self, type_id: TypeId, t: T) {
        if self.get(type_id).is_none() {
            self.0.push((type_id, t));
        }
    }
}

/// A list of types associated with an id
struct WatchedPath<T> {
    id: SharedString,
    types: Types<T>,
}

impl<T> WatchedPath<T> {
    const fn new(id: SharedString) -> Self {
        Self {
            id,
            types: Types::new(),
        }
    }
}

/// The list of watched paths.
///
/// Each type is associated with the function to load an asset of this type.
/// This is kept up to date by the matching `AssetCache`, which sends messages
/// when an asset or a directory is added.
pub struct AssetPaths {
    assets: HashMap<PathBuf, WatchedPath<LoadFn>>,
}

impl AssetPaths {
    fn clear(&mut self) {
        self.assets.clear();
    }

    fn add_asset(&mut self, id: AssetReloadInfos) {
        let AssetReloadInfos(path, id, type_id, load) = id;
        let watched = self
            .assets
            .entry(path)
            .or_insert_with(|| WatchedPath::new(id));
        watched.types.insert(type_id, load);
    }
}

/// Store assets until we can sync with the `AssetCache`.
pub struct LocalCache {
    changed: HashMap<OwnedKey, Box<dyn AnyAsset>>,
}

impl LocalCache {
    fn clear(&mut self) {
        self.changed.clear();
    }
}

enum CacheKind {
    Local(LocalCache),
    Static(&'static AssetCache, Vec<OwnedKey>),
}

impl CacheKind {
    /// Reloads an asset
    ///
    /// `key.type_id == asset.type_id()`
    fn update(&mut self, key: BorrowedKey, asset: Box<dyn AnyAsset>) {
        match self {
            CacheKind::Static(cache, to_reload) => {
                if let Some(entry) = cache.assets.get_entry(key) {
                    asset.reload(entry);
                    log::info!("Reloading \"{}\"", key.id());
                }
                to_reload.push(key.to_owned());
            }
            CacheKind::Local(cache) => {
                cache.changed.insert(key.to_owned(), asset);
            }
        }
    }
}

pub(crate) struct HotReloadingData {
    paths: AssetPaths,
    cache: CacheKind,
    deps: Dependencies,
}

impl HotReloadingData {
    pub fn new() -> Self {
        let cache = LocalCache {
            changed: HashMap::new(),
        };

        HotReloadingData {
            paths: AssetPaths {
                assets: HashMap::new(),
            },

            cache: CacheKind::Local(cache),
            deps: Dependencies::new(),
        }
    }

    /// A file was changed
    pub fn load(&mut self, path: PathBuf) -> Option<()> {
        let file_ext = extension_of(&path)?;

        self.load_asset(&path, file_ext);

        self.update_if_static();

        Some(())
    }

    fn load_asset(&mut self, path: &Path, file_ext: &str) {
        if let Some(path_infos) = self.paths.assets.get(path) {
            let content = match fs::read(path) {
                Ok(content) => content,
                Err(err) => {
                    log::warn!(
                        "Error reloading \"{}\" from \"{}\": {}",
                        path_infos.id,
                        path.display(),
                        err
                    );
                    return;
                }
            };

            for (type_id, load) in &path_infos.types.0 {
                if let Some(asset) = load(Cow::Borrowed(&content), file_ext, &path_infos.id, path) {
                    let key = BorrowedKey::new_with(&path_infos.id, *type_id);
                    self.cache.update(key, asset);
                }
            }
        }
    }

    pub fn update_if_local(&mut self, cache: &AssetCache) {
        if let CacheKind::Local(local_cache) = &mut self.cache {
            local_cache.update(&mut self.deps, cache);
        }
    }

    fn update_if_static(&mut self) {
        if let CacheKind::Static(cache, to_reload) = &mut self.cache {
            let to_update = super::dependencies::AssetDepGraph::new(&self.deps, to_reload.iter());
            to_update.update(&mut self.deps, cache);
            to_reload.clear();
        }
    }

    /// Drop the local cache and use the static reference we have on the
    /// `AssetCache`.
    pub fn use_static_ref(&mut self, asset_cache: &'static AssetCache) {
        if let CacheKind::Local(cache) = &mut self.cache {
            cache.update(&mut self.deps, asset_cache);
            self.cache = CacheKind::Static(asset_cache, Vec::new());
            log::trace!("Hot-reloading now use a 'static reference");
        }
    }

    pub fn recv_update(&mut self, message: UpdateMessage) {
        match message {
            UpdateMessage::Clear => {
                self.paths.clear();
                if let CacheKind::Local(cache) = &mut self.cache {
                    cache.clear();
                }
            }
            UpdateMessage::AddAsset(infos) => self.paths.add_asset(infos),
            UpdateMessage::AddCompound(infos) => {
                let CompoundReloadInfos(key, new_deps, reload) = infos;
                self.deps.insert(key, new_deps, Some(reload));
            }
        }
    }
}

impl LocalCache {
    /// Update the `AssetCache` with data collected in the `LocalCache` since
    /// the last reload.
    fn update(&mut self, deps: &mut Dependencies, cache: &AssetCache) {
        let to_update =
            super::dependencies::AssetDepGraph::new(deps, self.changed.iter().map(|(k, _)| k));

        // Update assets
        for (key, value) in self.changed.drain() {
            log::info!("Reloading \"{}\"", key.id());

            cache.assets.update_or_insert(
                key,
                value,
                |value, entry| value.reload(entry.inner()),
                |value, id| value.create(id),
            );
        }

        to_update.update(deps, cache);
    }
}
