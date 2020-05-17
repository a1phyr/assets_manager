use std::{
    any::{Any, TypeId},
    borrow::Cow,
    collections::{HashMap, HashSet},
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
};

use crate::RandomState;


fn borrowed(content: &io::Result<Vec<u8>>) -> io::Result<Cow<[u8]>> {
    match content {
        Ok(bytes) => Ok(bytes.into()),
        Err(err) => match err.raw_os_error() {
            Some(e) => Err(io::Error::from_raw_os_error(e)),
            None => Err(err.kind().into()),
        },
    }
}


trait AnyAsset: Any + Send + Sync {
    unsafe fn reload(self: Box<Self>, entry: &CacheEntry);
}

impl<A: Asset> AnyAsset for A {
    unsafe fn reload(self: Box<Self>, entry: &CacheEntry) {
        let asset: A = *self;
        entry.write(asset);
    }
}


type LoadFn = fn(content: io::Result<Cow<[u8]>>, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>>;

fn load<A: Asset>(content: io::Result<Cow<[u8]>>, id: &str, path: &Path) -> Option<Box<dyn AnyAsset>> {
    match A::Loader::load(content) {
        Ok(asset) => Some(Box::new(asset)),
        Err(e) => {
            log::warn!("Error reloading {:?} from {:?}: {}", id, path, e);
            None
        },
    }
}


struct WatchedPath {
    id: String,
    types: HashMap<TypeId, LoadFn, RandomState>,
}

impl WatchedPath {
    fn new(id: String) -> Self {
        Self {
            id,
            types: HashMap::with_hasher(RandomState::new()),
        }
    }
}

pub struct WatchedPaths {
    paths: HashMap<PathBuf, WatchedPath, RandomState>,
    added: HashSet<(PathBuf, TypeId), RandomState>,
    cleared: bool,
}

impl WatchedPaths {
    pub fn new() -> Self {
        Self {
            paths: HashMap::with_hasher(RandomState::new()),
            added: HashSet::with_hasher(RandomState::new()),
            cleared: false,
        }
    }

    pub fn add<A: Asset>(&mut self, path: PathBuf, id: String) {
        match self.paths.get_mut(&path) {
            None => {
                let mut types = HashMap::with_hasher(RandomState::new());
                types.insert(TypeId::of::<A>(), load::<A> as LoadFn);

                let info = WatchedPath { id, types };
                self.paths.insert(path.clone(), info);
            },
            Some(infos) => {
                debug_assert_eq!(infos.id, id);

                infos.types.insert(TypeId::of::<A>(), load::<A>);
            },
        }

        self.added.insert((path, TypeId::of::<A>()));
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.added.clear();
        self.cleared = true;
    }
}


pub struct FileCache {
    paths: HashMap<PathBuf, WatchedPath, RandomState>,
    changed: HashMap<Key, Box<dyn AnyAsset>, RandomState>,
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            paths: HashMap::with_hasher(RandomState::new()),
            changed: HashMap::with_hasher(RandomState::new()),
        }
    }

    pub fn load(&mut self, path: PathBuf) {
        let path_infos = match self.paths.get_mut(&path) {
            Some(i) => i,
            None => return,
        };

        let content = fs::read(&path);

        for (&type_id, load) in &mut path_infos.types {
            if let Some(asset) = load(borrowed(&content), &path_infos.id, &path) {
                let key = Key::new_with(path_infos.id.clone().into(), type_id);
                self.changed.insert(key, asset);

            }
        }
    }

    pub fn update(&mut self, cache: &AssetCache) {
        let assets = cache.assets.read();

        for (key, value) in self.changed.drain() {
            let mut changed = false;

            if let Some(entry) = assets.get(&key) {
                unsafe { value.reload(entry) };
                changed = true;
            }

            if changed {
                log::info!("Reloading {:?}", key.id());
            }
        }
    }

    pub fn get_watched(&mut self, watched: &mut WatchedPaths) {
        if watched.cleared {
            watched.cleared = false;
            self.paths.clear();
        }

        for (path, id) in watched.added.drain() {
            let infos = match watched.paths.get(&path) {
                Some(infos) => infos,
                None => continue,
            };

            let load = match infos.types.get(&id) {
                Some(&load) => load,
                None => continue,
            };

            let watched = self.paths.entry(path).or_insert_with(|| {
                WatchedPath::new(infos.id.clone())
            });

            watched.types.insert(id, load);
        }
    }
}
