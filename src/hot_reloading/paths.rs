use std::{
    any::{Any, TypeId},
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Asset,
    AssetCache,
    cache::{Key, OwnedKey},
    loader::Loader,
    entry::CacheEntry,
    utils::HashMap,
};


/// Push a component to an id
fn clone_and_push(id: &str, name: &str) -> Box<str> {
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
    fn create(self: Box<Self>) -> CacheEntry;
}

unsafe impl<A: Asset> AnyAsset for A {
    unsafe fn reload(self: Box<Self>, entry: &CacheEntry) {
        entry.write::<A>(*self);
    }

    fn create(self: Box<Self>) -> CacheEntry {
        CacheEntry::new::<A>(*self)
    }
}

type LoadFn = fn(content: Cow<[u8]>, ext: &str, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>>;

fn load<A: Asset>(content: Cow<[u8]>, ext: &str, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>> {
    match A::Loader::load(content, ext) {
        Ok(asset) => Some(Box::new(asset)),
        Err(err) => {
            log::warn!("Error reloading {:?} from {:?}: {}", id, path, err);
            None
        },
    }
}

type Ext = &'static [&'static str];

/// This struct is responsible of the safety of the whole module.
///
/// Its invariant is that the TypeId is the same as the one of the value
/// returned by the LoadFn.
pub struct Id(PathBuf, Box<str>, TypeId, LoadFn);

/// A update to the list of watched paths
#[non_exhaustive]
pub enum UpdateMessage {
    Clear,
    Asset(Id),
    Dir(Id, Ext),
}

impl UpdateMessage {
    #[inline]
    pub fn asset<A: Asset>(path: PathBuf, id: Box<str>) -> Self {
        let asset = Id(path, id, TypeId::of::<A>(), load::<A>);
        UpdateMessage::Asset(asset)
    }

    #[inline]
    pub fn dir<A: Asset>(path: PathBuf, id: Box<str>) -> Self {
        let dir = Id(path, id, TypeId::of::<A>(), load::<A>);
        UpdateMessage::Dir(dir, A::EXTENSIONS)
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
    id: Box<str>,
    types: Types<T>,
}

impl<T> WatchedPath<T> {
    const fn new(id: Box<str>) -> Self {
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
    /// Update the list given a message
    pub fn update(&mut self, msg: UpdateMessage) {
        match msg {
            UpdateMessage::Clear => {
                self.assets.clear();
                self.dirs.clear();
            },
            UpdateMessage::Asset(Id(path, id, type_id, load)) => {
                let watched = self.assets.entry(path).or_insert_with(|| WatchedPath::new(id));
                watched.types.insert(type_id, load);
            },
            UpdateMessage::Dir(Id(path, id, type_id, load), ext) => {
                let watched = self.dirs.entry(path).or_insert_with(|| WatchedPath::new(id));
                watched.types.insert(type_id, (load, ext));
            },
        }
    }
}


enum Action {
    Add,
    Remove,
}

/// Store assets until we can sync with the `AssetCache`.
pub struct LocalCache {
    changed: HashMap<OwnedKey, Box<dyn AnyAsset>>,
    changed_dirs: Vec<(OwnedKey, Box<str>, Action)>,
}

enum CacheKind {
    Local(LocalCache),
    Static(&'static AssetCache),
}

impl CacheKind {
    /// Reload an asset
    ///
    /// # Safety
    ///
    /// `key.type_id == asset.type_id()`
    unsafe fn update(&mut self, key: &Key, asset: Box<dyn AnyAsset>) {
        match self {
            CacheKind::Static(cache) => {
                log::info!("Reloading {:?}", key.id());

                let assets = cache.assets.read();
                if let Some(entry) = assets.get(key) {
                    asset.reload(entry);
                }
            },
            CacheKind::Local(cache) => {
                cache.changed.insert(key.to_owned(), asset);
            },
        }
    }

    /// Add an asset to a directory
    fn add(&mut self, dir_key: &Key, id: Box<str>) {
        match self {
            CacheKind::Static(cache) => {
                log::info!("Adding {:?} to {:?}", id, dir_key.id());

                let dirs = cache.dirs.read();
                if let Some(dir) = dirs.get(dir_key) {
                    dir.add(id);
                }
            },
            CacheKind::Local(cache) => {
                cache.changed_dirs.push((dir_key.to_owned(), id, Action::Add));
            },
        }
    }

    /// Remove an asset from a directory
    fn remove(&mut self, dir_key: &Key, id: Box<str>) {
        match self {
            CacheKind::Static(cache) => {
                log::info!("Removing {:?} from {:?}", id, dir_key.id());

                let dirs = cache.dirs.read();
                if let Some(dir) = dirs.get(dir_key) {
                    dir.remove(&id);
                }
            },
            CacheKind::Local(cache) => {
                cache.changed_dirs.push((dir_key.to_owned(), id, Action::Remove));
            },
        }
    }
}

pub struct HotReloadingData {
    pub paths: AssetPaths,
    cache: CacheKind,
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
        }
    }

    /// A file was changed
    pub fn load(&mut self, path: PathBuf) -> Option<()> {
        let file_ext = extension_of(&path)?;

        self.load_dir(&path, file_ext)?;
        self.load_asset(&path, file_ext);

        Some(())
    }

    fn load_asset(&mut self, path: &Path, file_ext: &str) {
        if let Some(path_infos) = self.paths.assets.get(path) {
            let content = match fs::read(path) {
                Ok(content) => content,
                Err(err) => {
                    log::warn!("Error reloading {:?} from {:?}: {}", path_infos.id, path, err);
                    return;
                }
            };

            for (type_id, load) in &path_infos.types.0 {
                if let Some(asset) = load(Cow::Borrowed(&content), file_ext, &path_infos.id, path) {
                    unsafe {
                        let key = Key::new_with(&path_infos.id, *type_id);
                        self.cache.update(&key, asset);
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
                    self.cache.add(&key, file_id);
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
                self.cache.remove(&key, id);
            }
        }

        Some(())
    }

    /// Drop the local cache and use the static reference we have on the
    /// `AssetCache`.
    pub fn use_static_ref(&mut self, asset_cache: &'static AssetCache) {
        if let CacheKind::Local(cache) = &mut self.cache {
            cache.update(asset_cache);
            self.cache = CacheKind::Static(asset_cache);
            log::trace!("Hot-reloading now use a 'static reference");
        }
    }

    pub fn local_cache(&mut self) -> Option<&mut LocalCache> {
        match &mut self.cache {
            CacheKind::Local(cache) => Some(cache),
            CacheKind::Static(_) => None,
        }
    }
}

impl LocalCache {
    /// Update the `AssetCache` with data collected in the `LocalCache` since
    /// the last reload.
    pub fn update(&mut self, cache: &AssetCache) {
        // Update assets
        let mut assets = cache.assets.write();

        for (key, value) in self.changed.drain() {
            log::info!("Reloading {:?}", key.id());

            use std::collections::hash_map::Entry::*;
            match assets.entry(key) {
                Occupied(entry) => unsafe { value.reload(entry.get()) },
                Vacant(entry) => { entry.insert(value.create()); },
            }

        }
        drop(assets);

        // Update directories
        let dirs = cache.dirs.read();

        for (key, id, action) in self.changed_dirs.drain(..) {
            match action {
                Action::Add => {
                    if let Some(dir) = dirs.get(&key) {
                        if !dir.contains(&id) {
                            log::info!("Adding {:?} to {:?}", id, key.id());
                            dir.add(id);
                        }
                    }
                }
                Action::Remove => {
                    if let Some(dir) = dirs.get(&key) {
                        log::info!("Removing {:?} from {:?}", id, key.id());
                        dir.remove(&id);
                    }
                }
            }
        }
    }
}
