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
    dirs::{extension_of, has_extension, id_push},
    loader::Loader,
    lock::CacheEntry,
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
    id_push(&mut id, name);
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

struct WatchedPath<T> {
    id: Box<str>,
    types: Types<T>,
}

impl<T> WatchedPath<T> {
    fn new(id: Box<str>) -> Self {
        Self {
            id,
            types: Types::new(),
        }
    }
}

pub struct WatchedPaths {
    files: HashMap<PathBuf, WatchedPath<LoadFn>, RandomState>,
    dirs: HashMap<PathBuf, WatchedPath<(LoadFn, Ext)>, RandomState>,

    added: Vec<(PathBuf, TypeId, bool)>,
    cleared: bool,
}

impl WatchedPaths {
    pub fn new() -> Self {
        Self {
            files: HashMap::with_hasher(RandomState::new()),
            dirs: HashMap::with_hasher(RandomState::new()),
            added: Vec::new(),
            cleared: false,
        }
    }

    fn _add_file(&mut self, path: PathBuf, id: Box<str>, load: LoadFn, type_id: TypeId) {
        let infos = match self.files.get_mut(&path) {
            None => {
                let info = WatchedPath::new(id);
                self.files.entry(path.clone()).or_insert(info)
            },
            Some(infos) => {
                debug_assert_eq!(infos.id, id);
                infos
            },
        };

        infos.types.insert(type_id, load);
        self.added.push((path, type_id, true));
    }

    fn _add_dir(&mut self, path: PathBuf, id: Box<str>, load: LoadFn, type_id: TypeId, ext: Ext) {
        let infos = match self.dirs.get_mut(&path) {
            None => {
                let info = WatchedPath::new(id);
                self.dirs.entry(path.clone()).or_insert(info)
            },
            Some(infos) => {
                debug_assert_eq!(infos.id, id);
                infos
            },
        };

        infos.types.insert(type_id, (load, ext));
        self.added.push((path, type_id, false));
    }

    #[inline]
    pub fn add_file<A: Asset>(&mut self, path: PathBuf, id: Box<str>) {
        self._add_file(path, id, load::<A>, TypeId::of::<A>());
    }

    #[inline]
    pub fn add_dir<A: Asset>(&mut self, path: PathBuf, id: Box<str>) {
        self._add_dir(path, id, load::<A>, TypeId::of::<A>(), A::EXTENSIONS);
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.dirs.clear();
        self.added.clear();
        self.cleared = true;
    }
}


pub struct FileCache {
    files: HashMap<PathBuf, WatchedPath<LoadFn>, RandomState>,
    dirs: HashMap<PathBuf, WatchedPath<(LoadFn, Ext)>, RandomState>,

    changed: HashMap<Key, Box<dyn AnyAsset>, RandomState>,
    added: Vec<(Key, Box<str>)>,
    removed: Vec<(Key, Box<str>)>,
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            files: HashMap::with_hasher(RandomState::new()),
            dirs: HashMap::with_hasher(RandomState::new()),

            changed: HashMap::with_hasher(RandomState::new()),
            added: Vec::new(),
            removed: Vec::new(),
        }
    }

    pub fn load(&mut self, path: PathBuf) {
        let ext = match extension_of(&path) {
            Some(ext) => ext,
            None => return,
        };

        match self.files.get(&path) {
            Some(path_infos) => {
                let content = fs::read(&path);

                for (type_id, load) in &path_infos.types.0 {
                    if let Some(asset) = load(borrowed(&content), ext, &path_infos.id, &path) {
                        let key = Key::new_with(path_infos.id.clone(), *type_id);
                        self.changed.insert(key, asset);
                    }
                }
            }
            None => {
                self.load_dir(path);
            },
        }
    }

    fn load_dir(&mut self, path: PathBuf) -> Option<()> {
        let file_ext = extension_of(&path)?;
        let parent = path.parent()?;
        let path_infos = self.dirs.get(parent)?;

        let file_stem = path.file_stem()?.to_str()?;

        for &(type_id, (load, ext)) in &path_infos.types.0 {
            if has_extension(&path, ext) {
                let key = Key::new_with(path_infos.id.clone(), type_id);
                let id = clone_and_push(&path_infos.id, file_stem);
                self.added.push((key, id));

                let content = fs::read(&path).map(Into::into);
                if let Some(asset) = load(content, file_ext, &path_infos.id, &path) {
                    let key = Key::new_with(path_infos.id.clone(), type_id);
                    self.changed.insert(key, asset);
                }
            }
        }

        Some(())
    }

    pub fn remove(&mut self, path: PathBuf) -> Option<()> {
        let parent = path.parent()?;
        let path_infos = self.dirs.get(parent)?;

        let file_stem = path.file_stem()?.to_str()?;

        for &(type_id, (_, ext)) in &path_infos.types.0 {
            if has_extension(&path, ext) {
                let key = Key::new_with(path_infos.id.clone(), type_id);
                let id = clone_and_push(&path_infos.id, file_stem);
                self.removed.push((key, id));
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

        for (key, id) in self.removed.drain(..) {
            if let Some(dir) = dirs.get(&key) {
                dir.remove(&id);
                log::info!("Removing {:?} from {:?}", id, key.id());
            }
        }

        for (key, id) in self.added.drain(..) {
            if let Some(dir) = dirs.get(&key) {
                log::info!("Adding {:?} to {:?}", id, key.id());
                dir.add(id);
            }
        }
    }

    pub fn get_watched(&mut self, watched: &mut WatchedPaths) {
        if watched.cleared {
            watched.cleared = false;
            self.files.clear();
            self.dirs.clear();
        }

        for (path, id, is_file) in watched.added.drain(..) {
            if is_file {
                let infos = match watched.files.get(&path) {
                    Some(infos) => infos,
                    None => continue,
                };

                if let Some(&load) = infos.types.get(id) {
                    let watched = self.files.entry(path).or_insert_with(|| {
                        WatchedPath::new(infos.id.clone())
                    });

                    watched.types.insert(id, load);
                }
            } else {
                let infos = match watched.dirs.get(&path) {
                    Some(infos) => infos,
                    None => continue,
                };

                if let Some(&load) = infos.types.get(id) {
                    let watched = self.dirs.entry(path).or_insert_with(|| {
                        WatchedPath::new(infos.id.clone())
                    });

                    watched.types.insert(id, load);
                }
            }
        }
    }
}
