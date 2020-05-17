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
    cache::AccessKey,
    loader::Loader,
    lock::CacheEntry,
};

use crate::RandomState;


type AnyBox = Box<dyn Any>;

fn borrowed(content: &io::Result<Vec<u8>>) -> io::Result<Cow<[u8]>> {
    match content {
        Ok(bytes) => Ok(bytes.into()),
        Err(err) => match err.raw_os_error() {
            Some(e) => Err(io::Error::from_raw_os_error(e)),
            None => Err(err.kind().into()),
        },
    }
}

fn load<A: Asset>(content: io::Result<Cow<[u8]>>, id: &str, path: &Path) -> Option<AnyBox> {
    match A::Loader::load(content) {
        Ok(asset) => Some(Box::new(asset)),
        Err(e) => {
            log::warn!("Error reloading {:?} from {:?}: {}", id, path, e);
            None
        },
    }
}

unsafe fn reload<A: Asset>(entry: &CacheEntry, asset: AnyBox) {
    debug_assert!(asset.is::<A>());
    let asset = Box::from_raw(Box::into_raw(asset) as *mut A);
    entry.write(*asset);
}

#[derive(Clone, Copy)]
struct TypeInfo {
    load: fn(io::Result<Cow<[u8]>>, id: &str, path: &Path) -> Option<AnyBox>,
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
    types: HashMap<TypeId, TypeInfo, RandomState>,
}

pub struct WatchedPaths {
    paths: HashMap<PathBuf, WatchedPath, RandomState>,
    added: HashSet<PathBuf, RandomState>,
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
    infos: TypeInfo,
    value: Option<AnyBox>,
}

impl From<TypeInfo> for Value {
    fn from(infos: TypeInfo) -> Self {
        Self {
            infos,
            value: None,
        }
    }
}

struct PathValues {
    id: String,
    values: HashMap<TypeId, Value, RandomState>,
}

pub struct FileCache {
    cache: HashMap<PathBuf, PathValues, RandomState>,
    changed: Vec<PathBuf>,
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::with_hasher(RandomState::new()),
            changed: Vec::new(),
        }
    }

    pub fn load(&mut self, path: PathBuf) {
        let infos = match self.cache.get_mut(&path) {
            Some(i) => i,
            None => return,
        };

        let content = fs::read(&path);
        let mut changed = false;

        for info in infos.values.values_mut() {
            if let Some(asset) = (info.infos.load)(borrowed(&content), &infos.id, &path) {
                info.value = Some(asset);
                changed = true;
                log::info!("Reloading {:?} from {:?}", infos.id, path);
            }
        }

        if changed {
            self.changed.push(path);
        }
    }

    pub fn update(&mut self, cache: &AssetCache) {
        let assets = cache.assets.read();

        for path in self.changed.drain(..) {
            let path = match self.cache.get_mut(&path) {
                Some(values) => values,
                None => continue,
            };

            for (id, info) in &mut path.values {
                if let Some(val) = info.value.take() {
                    let key = AccessKey::new_with(&path.id, *id);
                    if let Some(entry) = assets.get(&key) {
                        unsafe {
                            (info.infos.reload)(entry, val);
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
                values: infos.types.iter().map(|(&id, &ty)| (id, ty.into())).collect(),
            };

            self.cache.insert(path, values);
        }
    }
}
