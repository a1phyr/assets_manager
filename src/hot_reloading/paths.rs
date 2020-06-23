use std::{
    any::{Any, TypeId},
    borrow::Cow,
    collections::HashMap,
    fs,
    io,
    path::{Path, PathBuf},
};

use crate::{
    Asset,
    AssetCache,
    cache::Key,
    loader::Loader,
    lock::CacheEntry,
    source::extension_of,
};

use crate::RandomState;


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


fn borrowed(content: &io::Result<Vec<u8>>) -> io::Result<Cow<[u8]>> {
    match content {
        Ok(bytes) => Ok(bytes.into()),
        Err(err) => match err.raw_os_error() {
            Some(e) => Err(io::Error::from_raw_os_error(e)),
            None => Err(err.kind().into()),
        },
    }
}

fn clone_and_push(id: &str, name: &str) -> Box<str> {
    let mut id = id.to_string();
    if !id.is_empty() {
        id.push('.');
    }
    id.push_str(name);
    id.into()
}


trait AnyAsset: Any + Send + Sync {
    unsafe fn reload(self: Box<Self>, entry: &CacheEntry);
    fn create(self: Box<Self>) -> CacheEntry;
}

impl<A: Asset> AnyAsset for A {
    unsafe fn reload(self: Box<Self>, entry: &CacheEntry) {
        entry.write::<A>(*self);
    }

    fn create(self: Box<Self>) -> CacheEntry {
        CacheEntry::new::<A>(*self)
    }
}


type LoadFn = fn(content: io::Result<Cow<[u8]>>, ext: &str, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>>;

fn load<A: Asset>(content: io::Result<Cow<[u8]>>, ext: &str, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>> {
    match A::Loader::load(content, ext) {
        Ok(asset) => Some(Box::new(asset)),
        Err(e) => {
            log::warn!("Error reloading {:?} from {:?}: {}", id, path, e);
            None
        },
    }
}

type Ext = &'static [&'static str];

enum Kind {
    Asset,
    Dir(Ext),
}

pub struct WatchedPaths {
    added: Vec<(PathBuf, Box<str>, TypeId, LoadFn, Kind)>,
    cleared: bool,
}

impl WatchedPaths {
    pub fn new() -> Self {
        Self {
            added: Vec::new(),
            cleared: false,
        }
    }

    #[inline]
    pub fn add_file<A: Asset>(&mut self, path: PathBuf, id: Box<str>) {
        self.added.push((path, id, TypeId::of::<A>(), load::<A>, Kind::Asset));
    }

    #[inline]
    pub fn add_dir<A: Asset>(&mut self, path: PathBuf, id: Box<str>) {
        self.added.push((path, id, TypeId::of::<A>(), load::<A>, Kind::Dir(A::EXTENSIONS)));
    }

    #[inline]
    pub fn clear(&mut self) {
        self.added.clear();
        self.cleared = true;
    }
}

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

enum Action {
    Added,
    Removed,
}

pub struct FileCache {
    assets: HashMap<PathBuf, WatchedPath<LoadFn>, RandomState>,
    dirs: HashMap<PathBuf, WatchedPath<(LoadFn, Ext)>, RandomState>,

    changed: HashMap<Key, Box<dyn AnyAsset>, RandomState>,
    changed_dirs: Vec<(Key, Box<str>, Action)>,
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            assets: HashMap::with_hasher(RandomState::new()),
            dirs: HashMap::with_hasher(RandomState::new()),

            changed: HashMap::with_hasher(RandomState::new()),
            changed_dirs: Vec::new(),
        }
    }

    pub fn load(&mut self, path: PathBuf) -> Option<()> {
        let file_ext = extension_of(&path)?;

        self.load_dir(&path, file_ext)?;
        self.load_asset(&path, file_ext);

        Some(())
    }

    fn load_asset(&mut self, path: &Path, file_ext: &str) {
        if let Some(path_infos) = self.assets.get(path) {
            let content = fs::read(path);

            for (type_id, load) in &path_infos.types.0 {
                if let Some(asset) = load(borrowed(&content), file_ext, &path_infos.id, path) {
                    let key = Key::new_with(path_infos.id.clone(), *type_id);
                    self.changed.insert(key, asset);
                }
            }
        }
    }

    fn load_dir(&mut self, path: &Path, file_ext: &str) -> Option<()> {
        let parent = path.parent()?;
        let file_stem = path.file_stem()?.to_str()?;

        if let Some(path_infos) = self.dirs.get(parent) {
            for &(type_id, (load, type_ext)) in &path_infos.types.0 {
                if type_ext.contains(&file_ext) {
                    let key = Key::new_with(path_infos.id.clone(), type_id);
                    let file_id = clone_and_push(&path_infos.id, file_stem);

                    let watched = self.assets.entry(path.into()).or_insert_with(|| WatchedPath::new(file_id.clone()));
                    watched.types.insert(type_id, load);

                    self.changed_dirs.push((key, file_id, Action::Added));
                }
            }
        }

        Some(())
    }

    pub fn remove(&mut self, path: PathBuf) -> Option<()> {
        let parent = path.parent()?;
        let path_infos = self.dirs.get(parent)?;
        let file_ext = extension_of(&path)?;

        let file_stem = path.file_stem()?.to_str()?;

        for &(type_id, (_, type_ext)) in &path_infos.types.0 {
            if type_ext.contains(&file_ext) {
                let key = Key::new_with(path_infos.id.clone(), type_id);
                let id = clone_and_push(&path_infos.id, file_stem);
                self.changed_dirs.push((key, id, Action::Removed));
            }
        }

        Some(())
    }

    pub fn update(&mut self, cache: &AssetCache) {
        let mut assets = cache.assets.write();

        for (key, value) in self.changed.drain() {
            log::info!("Reloading {:?}", key.id());

            use std::collections::hash_map::Entry::*;
            match assets.entry(key) {
                Occupied(entry) => unsafe { value.reload(entry.get()) },
                Vacant(entry) =>  { entry.insert(value.create()); },
            }

        }
        drop(assets);

        let dirs = cache.dirs.read();

        for (key, id, action) in self.changed_dirs.drain(..) {
            match action {
                Action::Added => {
                    if let Some(dir) = dirs.get(&key) {
                        if !dir.contains(&id) {
                            log::info!("Adding {:?} to {:?}", id, key.id());
                            dir.add(id);
                        }
                    }
                }
                Action::Removed => {
                    if let Some(dir) = dirs.get(&key) {
                        log::info!("Removing {:?} from {:?}", id, key.id());
                        dir.remove(&id);
                    }
                }
            }
        }
    }

    pub fn get_watched(&mut self, watched: &mut WatchedPaths) {
        if watched.cleared {
            watched.cleared = false;
            watched.added.clear();
        }

        for (path, id, type_id, load, kind) in watched.added.drain(..) {
            match kind {
                Kind::Asset => {
                    let watched = self.assets.entry(path).or_insert_with(|| WatchedPath::new(id));
                    watched.types.insert(type_id, load);
                },
                Kind::Dir(ext) => {
                    let watched = self.dirs.entry(path).or_insert_with(|| WatchedPath::new(id));
                    watched.types.insert(type_id, (load, ext));
                },
            }
        }
    }
}
