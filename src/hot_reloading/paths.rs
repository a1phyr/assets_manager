use std::{
    any::{Any, TypeId},
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    Asset,
    AssetCache,
    Compound,
    loader::Loader,
    entry::CacheEntry,
    utils::{BorrowedKey, HashMap, HashSet, Key, OwnedKey},
};

use super::dependencies::Dependencies;


/// Push a component to an id
fn clone_and_push(id: &str, name: &str) -> Arc<str> {
    let mut id = id.to_string();
    if !id.is_empty() {
        id.push('.');
    }
    id.push_str(name);
    id.into()
}

#[inline]
fn extension_of(path: &Path) -> Option<&str> {
    match path.extension() {
        Some(ext) => ext.to_str(),
        None => Some(""),
    }
}


unsafe trait AnyAsset: Any + Send + Sync {
    unsafe fn reload(self: Box<Self>, entry: &CacheEntry);
    fn create(self: Box<Self>, id: Arc<str>) -> CacheEntry;
}

unsafe impl<A: Asset> AnyAsset for A {
    unsafe fn reload(self: Box<Self>, entry: &CacheEntry) {
        let handle = entry.handle::<A>();
        handle.either(
            |_| log::error!("Static asset registered for hot-reloading: {}", std::any::type_name::<A>()),
            |e| e.write(*self),
        );
    }

    fn create(self: Box<Self>, id: Arc<str>) -> CacheEntry {
        CacheEntry::new::<A>(*self, id)
    }
}

type LoadFn = fn(content: Cow<[u8]>, ext: &str, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>>;

fn load<A: Asset>(content: Cow<[u8]>, ext: &str, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>> {
    match A::Loader::load(content, ext) {
        Ok(asset) => Some(Box::new(asset)),
        Err(err) => {
            log::warn!("Error reloading \"{}\" from \"{}\": {}", id, path.display(), err);
            None
        },
    }
}

pub(crate) type ReloadFn = fn(cache: &AssetCache, id: &str) -> Option<HashSet<OwnedKey>>;

fn reload<T: Compound>(cache: &AssetCache, id: &str) -> Option<HashSet<OwnedKey>> {
    let key: &dyn Key = &Key::new::<T>(id);
    let handle = unsafe { cache.assets.read().get(key)?.handle::<T>() };
    let entry = handle.either(
        |_| {
            log::error!("Static asset registered for hot-reloading: {}", std::any::type_name::<T>());
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


type Ext = &'static [&'static str];

/// This struct is responsible of the safety of the whole module.
///
/// Its invariant is that the TypeId is the same as the one of the value
/// returned by the LoadFn.
pub(crate) struct AssetReloadInfos(PathBuf, Arc<str>, TypeId, LoadFn);

impl AssetReloadInfos {
    #[inline]
    pub fn of<A: Asset>(path: PathBuf, id: Arc<str>) -> Self {
        AssetReloadInfos(path, id, TypeId::of::<A>(), load::<A>)
    }
}

pub(crate) struct CompoundReloadInfos(OwnedKey, HashSet<OwnedKey>, ReloadFn);

impl CompoundReloadInfos {
    #[inline]
    pub fn of<A: Compound>(id: Arc<str>, deps: HashSet<OwnedKey>) -> Self {
        let key = OwnedKey::new::<A>(id);
        CompoundReloadInfos(key, deps, reload::<A>)
    }
}

/// A update to the list of watched paths
#[non_exhaustive]
pub(crate) enum UpdateMessage {
    Clear,
    AddAsset(AssetReloadInfos),
    AddDir(AssetReloadInfos, Ext),
    AddCompound(CompoundReloadInfos),
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
    id: Arc<str>,
    types: Types<T>,
}

impl<T> WatchedPath<T> {
    const fn new(id: Arc<str>) -> Self {
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
    dirs: HashMap<PathBuf, WatchedPath<(LoadFn, Ext)>>,
}

impl AssetPaths {
    fn clear(&mut self) {
        self.assets.clear();
        self.dirs.clear();
    }

    fn add_asset(&mut self, id: AssetReloadInfos) {
        let AssetReloadInfos(path, id, type_id, load) = id;
        let watched = self.assets.entry(path).or_insert_with(|| WatchedPath::new(id));
        watched.types.insert(type_id, load);
    }

    fn add_dir(&mut self, id: AssetReloadInfos, ext: Ext) {
        let AssetReloadInfos(path, id, type_id, load) = id;
        let watched = self.dirs.entry(path).or_insert_with(|| WatchedPath::new(id));
        watched.types.insert(type_id, (load, ext));
    }
}


enum Action {
    Add,
    Remove,
}

/// Store assets until we can sync with the `AssetCache`.
pub struct LocalCache {
    changed: HashMap<OwnedKey, Box<dyn AnyAsset>>,
    changed_dirs: Vec<(OwnedKey, Arc<str>, Action)>,
}

impl LocalCache {
    fn clear(&mut self) {
        self.changed.clear();
        self.changed_dirs.clear();
    }
}

enum CacheKind {
    Local(LocalCache),
    Static(&'static AssetCache, Vec<OwnedKey>),
}

impl CacheKind {
    /// Reload an asset
    ///
    /// # Safety
    ///
    /// `key.type_id == asset.type_id()`
    unsafe fn update(&mut self, key: BorrowedKey, asset: Box<dyn AnyAsset>) {
        match self {
            CacheKind::Static(cache, to_reload) => {
                let dyn_key: &dyn Key = &key;
                let assets = cache.assets.read();
                if let Some(entry) = assets.get(dyn_key) {
                    asset.reload(entry);
                    log::info!("Reloading \"{}\"", key.id());
                }
                to_reload.push(key.to_owned());
            },
            CacheKind::Local(cache) => {
                cache.changed.insert(key.to_owned(), asset);
            },
        }
    }

    /// Add an asset to a directory
    fn add(&mut self, dir_key: BorrowedKey, id: Arc<str>) {
        match self {
            CacheKind::Static(cache, _) => {
                let dir_key: &dyn Key = &dir_key;
                let dirs = cache.dirs.read();
                if let Some(dir) = dirs.get(dir_key) {
                    if dir.add(&id) {
                        log::info!("Adding \"{}\" to \"{}\"", id, dir_key.id());
                    }
                }
            },
            CacheKind::Local(cache) => {
                cache.changed_dirs.push((dir_key.to_owned(), id, Action::Add));
            },
        }
    }

    /// Remove an asset from a directory
    fn remove(&mut self, dir_key: BorrowedKey, id: Arc<str>) {
        match self {
            CacheKind::Static(cache, _) => {
                let dir_key: &dyn Key = &dir_key;
                let dirs = cache.dirs.read();
                if let Some(dir) = dirs.get(dir_key) {
                    if dir.remove(&id) {
                        log::info!("Removing \"{}\" from \"{}\"", id, dir_key.id());
                    }
                }
            },
            CacheKind::Local(cache) => {
                cache.changed_dirs.push((dir_key.to_owned(), id, Action::Remove));
            },
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
            changed_dirs: Vec::new(),
        };

        HotReloadingData {
            paths: AssetPaths {
                assets: HashMap::new(),
                dirs: HashMap::new(),
            },

            cache: CacheKind::Local(cache),
            deps: Dependencies::new(),
        }
    }

    /// A file was changed
    pub fn load(&mut self, path: PathBuf) -> Option<()> {
        let file_ext = extension_of(&path)?;

        self.load_dir(&path, file_ext)?;
        self.load_asset(&path, file_ext);

        self.update_if_static();

        Some(())
    }

    fn load_asset(&mut self, path: &Path, file_ext: &str) {
        if let Some(path_infos) = self.paths.assets.get(path) {
            let content = match fs::read(path) {
                Ok(content) => content,
                Err(err) => {
                    log::warn!("Error reloading \"{}\" from \"{}\": {}", path_infos.id, path.display(), err);
                    return;
                }
            };

            for (type_id, load) in &path_infos.types.0 {
                if let Some(asset) = load(Cow::Borrowed(&content), file_ext, &path_infos.id, path) {
                    unsafe {
                        let key = Key::new_with(&path_infos.id, *type_id);
                        self.cache.update(key, asset);
                    }
                }
            }
        }
    }

    fn load_dir(&mut self, path: &Path, file_ext: &str) -> Option<()> {
        let parent = path.parent()?;
        let file_stem = path.file_stem()?.to_str()?;

        if let Some(path_infos) = self.paths.dirs.get(parent) {
            for &(type_id, (load, type_ext)) in &path_infos.types.0 {
                if type_ext.contains(&file_ext) {
                    let file_id = clone_and_push(&path_infos.id, file_stem);

                    let watched = self.paths.assets.entry(path.into()).or_insert_with(|| WatchedPath::new(file_id.clone()));
                    watched.types.insert(type_id, load);

                    let key = Key::new_with(&path_infos.id, type_id);
                    self.cache.add(key, file_id);
                }
            }
        }

        Some(())
    }

    pub fn remove(&mut self, path: PathBuf) -> Option<()> {
        let parent = path.parent()?;
        let path_infos = self.paths.dirs.get(parent)?;
        let file_ext = extension_of(&path)?;

        let file_stem = path.file_stem()?.to_str()?;

        for &(type_id, (_, type_ext)) in &path_infos.types.0 {
            if type_ext.contains(&file_ext) {
                let key = Key::new_with(&path_infos.id, type_id);
                let id = clone_and_push(&path_infos.id, file_stem);
                self.cache.remove(key, id);
            }
        }

        Some(())
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
            },
            UpdateMessage::AddAsset(infos) => self.paths.add_asset(infos),
            UpdateMessage::AddDir(infos, ext) => self.paths.add_dir(infos, ext),
            UpdateMessage::AddCompound(infos) => {
                let CompoundReloadInfos(key, new_deps, reload) = infos;
                self.deps.insert(key, new_deps, Some(reload));
            },
        }
    }
}

impl LocalCache {
    /// Update the `AssetCache` with data collected in the `LocalCache` since
    /// the last reload.
    fn update(&mut self, deps: &mut Dependencies, cache: &AssetCache) {
        let to_update = super::dependencies::AssetDepGraph::new(&deps, self.changed.iter().map(|(k,_)| k));

        // Update assets
        let mut assets = cache.assets.write();

        for (key, value) in self.changed.drain() {
            log::info!("Reloading \"{}\"", key.id());

            use std::collections::hash_map::Entry::*;
            match assets.entry(key) {
                Occupied(entry) => unsafe { value.reload(entry.get()) },
                Vacant(entry) => {
                    let id = entry.key().id().into();
                    entry.insert(value.create(id));
                },
            }

        }
        drop(assets);

        // Update directories
        let dirs = cache.dirs.read();

        for (key, id, action) in self.changed_dirs.drain(..) {
            match action {
                Action::Add => {
                    if let Some(dir) = dirs.get(&key) {
                        if dir.add(&id) {
                            log::info!("Adding \"{}\" to \"{}\"", id, key.id());
                        }
                    }
                }
                Action::Remove => {
                    if let Some(dir) = dirs.get(&key) {
                        if dir.remove(&id) {
                            log::info!("Removing \"{}\" from \"{}\"", id, key.id());
                        }
                    }
                }
            }
        }

        to_update.update(deps, cache);
    }
}
