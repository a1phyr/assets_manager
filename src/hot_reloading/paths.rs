use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    fs,
    ops::Bound,
    path::PathBuf,
};

use crate::{
    Asset,
    AssetCache,
    AssetError,
    cache::AccessKey,
    loader::Loader,
    lock::CacheEntry,
};


const fn unbounded<T>() -> (Bound<T>, Bound<T>) {
    (Bound::Unbounded, Bound::Unbounded)
}


type AnyBox = Box<dyn Any + Send + Sync>;

fn load<A: Asset>(content: Vec<u8>) -> Result<AnyBox, AssetError> {
    match A::Loader::load(content) {
        Ok(asset) => Ok(Box::new(asset)),
        Err(e) => Err(AssetError::LoadError(e)),
    }
}

unsafe fn reload<A: Asset>(entry: &CacheEntry, asset: AnyBox) {
    let asset = Box::from_raw(Box::into_raw(asset) as *mut A);
    entry.write(*asset);
}

struct TypeInfo {
    load: fn(Vec<u8>) -> Result<AnyBox, AssetError>,
    reload: unsafe fn(&CacheEntry, AnyBox),
}

impl TypeInfo {
    fn of<A: Asset>() -> Self {
        Self {
            load: load::<A>,
            reload: reload::<A>,
        }
    }
}

struct WatchedPath {
    id: String,
    types: HashMap<TypeId, TypeInfo>,
}

pub struct WatchedPaths {
    paths: HashMap<PathBuf, WatchedPath>,
    added: HashSet<PathBuf>,
    cleared: bool,
}

impl WatchedPaths {
    pub fn new() -> Self {
        Self {
            paths: HashMap::new(),
            added: HashSet::new(),
            cleared: false,
        }
    }

    pub fn add<A: Asset>(&mut self, path: PathBuf, id: String) {
        match self.paths.get_mut(&path) {
            None => {
                let mut types = HashMap::new();
                types.insert(TypeId::of::<A>(), TypeInfo::of::<A>());

                let info = WatchedPath { id, types };
                self.paths.insert(path.clone(), info);
            },
            Some(infos) => {
                debug_assert_eq!(infos.id, id);

                infos.types.insert(TypeId::of::<A>(), TypeInfo::of::<A>());
            },
        }

        self.added.insert(path);
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.added.clear();
        self.cleared = true;
    }
}

struct Value {
    load: fn(Vec<u8>) -> Result<AnyBox, AssetError>,
    reload: unsafe fn(&CacheEntry, AnyBox),
    value: Option<AnyBox>,
}

impl From<&TypeInfo> for Value {
    fn from(infos: &TypeInfo) -> Self {
        Self {
            load: infos.load,
            reload: infos.reload,
            value: None,
        }
    }
}

struct PathValues {
    id: String,
    values: HashMap<TypeId, Value>,
}

pub struct FileCache {
    cache: HashMap<PathBuf, PathValues>,
    changed: Vec<PathBuf>,
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            changed: Vec::new(),
        }
    }

    pub fn load(&mut self, path: PathBuf) {
        let infos = match self.cache.get_mut(&path) {
            Some(i) => i,
            None => return,
        };

        let content = match fs::read(&path) {
            Ok(content) => content,
            Err(e) => {
                log::warn!("Error reloading {:?} from {:?}: {}", infos.id, path, e);
                return;
            },
        };

        let mut changed = false;

        for info in infos.values.values_mut() {
            match (info.load)(content.clone()) {
                Ok(asset) => {
                    info.value = Some(asset);
                    changed = true;
                    log::info!("Reloading {:?} from {:?}", infos.id, path);
                },
                Err(e) => log::warn!("Error reloading {:?} from {:?}: {}", infos.id, path, e),
            }
        }

        if changed {
            self.changed.push(path);
        }
    }

    pub fn update(&mut self, cache: &AssetCache) {
        let assets = cache.assets.read();

        for path in self.changed.drain(unbounded::<usize>()) {
            let path = match self.cache.get_mut(&path) {
                Some(values) => values,
                None => continue,
            };

            for (id, info) in &mut path.values {
                if let Some(val) = info.value.take() {
                    let key = AccessKey::new_with(&path.id, *id);
                    if let Some(entry) = assets.get(&key) {
                        unsafe {
                            (info.reload)(entry, val);
                        }
                    }
                }
            }
        }
    }

    pub fn get_watched(&mut self, watched: &mut WatchedPaths) {
        if watched.cleared {
            watched.cleared = false;
            self.cache.clear();
        }

        for path in watched.added.drain() {
            let infos = match watched.paths.get(&path) {
                Some(infos) => infos,
                None => {
                    debug_assert!(false);
                    continue;
                },
            };

            let values = PathValues {
                id: infos.id.clone(),
                values: infos.types.iter().map(|(id, ty)| (*id, ty.into())).collect(),
            };

            self.cache.insert(path, values);
        }
    }
}
