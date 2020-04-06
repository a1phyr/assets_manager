//! Definition of the cache
use crate::{
    Asset,
    AssetError,
    lock::{RwLock, CacheEntry, AssetRefLock},
};

#[cfg(feature = "hot-reloading")]
use crate::{
    lock::Mutex,
    hot_reloading::HotReloader,
};

use std::{
    any::TypeId,
    borrow::Borrow,
    cmp::Ordering,
    collections::BTreeMap,
    fmt,
    fs,
    io,
    path::{Path, PathBuf},
};

#[cfg(feature = "hot-reloading")]
use std::ops::Range;


#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum TypeIdExt {
    #[cfg(feature = "hot-reloading")]
    Min,
    Id(TypeInfo),
    #[cfg(feature = "hot-reloading")]
    Max,
}

impl TypeIdExt {
    fn of<A: Asset>() -> Self {
        Self::Id(TypeInfo {
            id: TypeId::of::<A>(),
            #[cfg(feature = "hot-reloading")]
            ext: A::EXT,
            #[cfg(feature = "hot-reloading")]
            reload: reload_one::<A>,
        })
    }

    fn unwrap(&self) -> &TypeInfo {
        match self {
            Self::Id(id) => id,
            #[cfg(feature = "hot-reloading")]
            _ => panic!(),
        }
    }
}

struct TypeInfo {
    id: TypeId,
    #[cfg(feature = "hot-reloading")]
    ext: &'static str,
    #[cfg(feature = "hot-reloading")]
    reload: unsafe fn(&CacheEntry, Vec<u8>) -> Result<(), AssetError>,
}

impl PartialEq for TypeInfo {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for TypeInfo {}

impl PartialOrd for TypeInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.id.partial_cmp(&other.id)
    }
}

impl Ord for TypeInfo {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

/// The key used to identify assets
///
/// **Note**: This definition has to kept in sync with [`AccessKey`]'s one.
///
/// [`AccessKey`]: struct.AccessKey.html
#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
struct Key {
    id: Box<str>,
    type_id: TypeIdExt,
}

impl Key {
    /// Creates a Key with the given type and id.
    #[inline]
    fn new<T: Asset>(id: Box<str>) -> Self {
        Self {
            id,
            type_id: TypeIdExt::of::<T>(),
        }
    }
}

/// A borrowed version of [`Key`]
///
/// [`Key`]: struct.Key.html
#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
struct AccessKey<'a> {
    id: &'a str,
    type_id: TypeIdExt,
}

impl<'a> AccessKey<'a> {
    /// Creates an AccessKey for the given type and id.
    #[inline]
    fn new<T: Asset>(id: &'a str) -> Self {
        Self {
            id,
            type_id: TypeIdExt::of::<T>(),
        }
    }

    #[cfg(feature = "hot-reloading")]
    fn range_of(id: &'a str) -> Range<Self> {
        Range {
            start: Self { id, type_id: TypeIdExt::Min },
            end: Self { id, type_id: TypeIdExt::Max },
        }
    }
}

impl<'a> Borrow<AccessKey<'a>> for Key {
    #[inline]
    fn borrow(&self) -> &AccessKey<'a> {
        unsafe {
            let ptr = self as *const Key as *const AccessKey;
            &*ptr
        }
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key: &AccessKey = self.borrow();
        key.fmt(f)
    }
}

impl fmt::Debug for AccessKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("id", &self.id)
            .field("type_id", &self.type_id.unwrap().id)
            .finish()
    }
}

/// The main structure of this crate, used to cache assets.
///
/// It uses interior mutability, so assets can be added in the cache without
/// requiring a mutable reference, but one is required to remove an asset.
///
/// Within the cache, assets are identified with their type and a string. This
/// string is constructed from the asset path, remplacing `/` by `.` and removing
/// the extension.
///
/// # Example
///
/// ```
/// # cfg_if::cfg_if! { if #[cfg(feature = "ron")] {
/// use assets_manager::{Asset, AssetCache, loader};
/// use serde::Deserialize;
///
/// #[derive(Debug, Deserialize)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
///
/// impl Asset for Point {
///     const EXT: &'static str = "ron";
///     type Loader = loader::RonLoader;
/// }
///
/// // Create a cache
/// let cache = AssetCache::new("assets")?;
///
/// // Get an asset from the file `assets/common/position.ron`
/// let point_lock = cache.load::<Point>("common.position")?;
///
/// // Read it
/// let point = point_lock.read();
/// println!("Loaded position: {:?}", point);
/// # assert_eq!(point.x, 5);
/// # assert_eq!(point.y, -6);
///
/// // Release the lock
/// drop(point);
///
/// // Use hot-reloading
/// loop {
///     println!("Position: {:?}", point_lock.read());
/// #   #[cfg(feature = "hot-reloading")]
///     cache.hot_reload();
/// #   break;
/// }
///
/// # }}
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct AssetCache {
    assets: RwLock<BTreeMap<Key, CacheEntry>>,
    path: PathBuf,

    #[cfg(feature = "hot-reloading")]
    reloader: Mutex<Option<HotReloader>>,
}

impl AssetCache {
    /// Creates a new cache.
    ///
    /// Assets will be searched in the directory given by `path`. Symbolic links
    /// will be followed.
    ///
    /// # Errors
    ///
    /// An error will be returned if `path` is not valid readable directory.
    #[inline]
    pub fn new<P: AsRef<Path>>(path: P) -> Result<AssetCache, io::Error> {
        let path = path.as_ref().canonicalize()?;
        let _ = path.read_dir()?;

        Ok(AssetCache {
            assets: RwLock::new(BTreeMap::new()),
            path,

            #[cfg(feature = "hot-reloading")]
            reloader: Mutex::new(None),
        })
    }

    /// Gets the path of the cache's root.
    ///
    /// The path is currently given as absolute, but this may change in the future.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Adds an asset to the cache
    pub(crate) fn add_asset<A: Asset>(&self, id: String, asset: A) -> AssetRefLock<A> {
        let entry = CacheEntry::new(asset);
        // Safety:
        // We just created the asset with the good type
        // The cache entry is garantied to live long enough
        let asset = unsafe { entry.get_ref() };

        let key = Key::new::<A>(id.into());
        let mut cache = self.assets.write();
        cache.insert(key, entry);

        asset
    }

    /// Loads an asset.
    ///
    /// If the asset is not found in the cache, it is loaded from the filesystem.
    ///
    /// # Errors
    ///
    /// Errors can occur in several cases :
    /// - The asset could not be loaded from the filesystem
    /// - Loaded data could not not be converted properly
    pub fn load<A: Asset>(&self, id: &str) -> Result<AssetRefLock<A>, AssetError> {
        if let Some(asset) = self.load_cached(id) {
            return Ok(asset);
        }

        let asset = self.load_from_fs(id)?;
        Ok(self.add_asset(id.to_string(), asset))
    }

    /// Loads an asset from the cache.
    ///
    /// This function does not attempt to load the asset from the filesystem if
    /// it is not found in the cache.
    pub fn load_cached<A: Asset>(&self, id: &str) -> Option<AssetRefLock<A>> {
        let key = AccessKey::new::<A>(id);
        let cache = self.assets.read();
        cache.get(&key).map(|asset| unsafe { asset.get_ref() })
    }

    /// Loads an asset given an id, from the filesystem or the cache.
    ///
    /// # Panics
    ///
    /// Panics if an error happens while loading the asset (see [`load`]).
    ///
    /// [`load`]: fn.load.html
    #[inline]
    pub fn load_expect<A: Asset>(&self, id: &str) -> AssetRefLock<A> {
        self.load(id).expect("Could not load essential asset")
    }

    /// Reloads an asset from the filesystem.
    ///
    /// It does not matter whether the asset has been loaded yet.
    ///
    /// **Note**: this function requires a write lock on the asset, and will block
    /// until one is aquired, ie no read lock can exist at the same time. This
    /// means that you **must not** call this method if you have an `AssetRef`
    /// on the same asset, or it may cause a deadlock.
    ///
    /// # Errors
    ///
    /// Error cases are the same as [`load`].
    ///
    /// If an error occurs, the asset is left unmodified.
    ///
    /// [`load`]: fn.load.html
    pub fn force_reload<A: Asset>(&self, id: &str) -> Result<AssetRefLock<A>, AssetError> {
        let asset = self.load_from_fs(id)?;

        let cache = self.assets.read();
        if let Some(cached) = cache.get(&AccessKey::new::<A>(id)) {
            return unsafe { Ok(cached.write(asset)) };
        }
        drop(cache);

        Ok(self.add_asset(id.to_string(), asset))
    }


    fn load_from_fs<A: Asset>(&self, id: &str) -> Result<A, AssetError> {
        let mut path = self.path.clone();
        path.extend(id.split('.'));
        path.set_extension(A::EXT);

        let content = fs::read(&path)?;
        A::load_from_raw(content)
    }

    /// Remove an asset from the cache.
    ///
    /// The removed asset matches both the id and the type parameter.
    #[inline]
    pub fn remove<A: Asset>(&mut self, id: &str) {
        let key = AccessKey::new::<A>(id);
        let cache = self.assets.get_mut();
        cache.remove(&key);
    }

    /// Take ownership on an asset.
    ///
    /// The corresponding asset is removed from the cache.
    pub fn take<A: Asset>(&mut self, id: &str) -> Option<A> {
        let key = AccessKey::new::<A>(id);
        let cache = self.assets.get_mut();
        cache.remove(&key).map(|entry| unsafe { entry.into_inner() })
    }

    /// Clears the cache.
    #[inline]
    pub fn clear(&mut self) {
        self.assets.get_mut().clear();
    }

    /// Reloads changed assets.
    ///
    /// The first time this function is called, the hot-reloading is started.
    /// Next calls to this function update assets if the cache if their related
    /// file have changed since the last call to this function.
    ///
    /// This function is typically called within a loop.
    ///
    /// If an error occurs while reloading an asset, a warning will be logged
    /// and the asset will be left unchanged.
    ///
    /// This function will block the current thread until all changed assets are
    /// reloaded, but it does not perform any I/O. However, it will need to lock
    /// some assets for writing, so you **must not** have any [`AssetRef`] from
    /// the given `AssetCache`, or you might experience deadlocks. You are free
    /// to keep [`AssetRefLock`]s, though.
    ///
    /// [`AssetRef`]: struct.AssetRef.html
    /// [`AssetRefLock`]: struct.AssetRefLock.html
    ///
    /// # Errors
    ///
    /// This function will return an error it it failed to start hot-reloading.
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    pub fn hot_reload(&self) -> Result<(), notify::Error> {
        let mut reloader = self.reloader.lock();
        match &*reloader {
            Some(reloader) => reloader.reload(self),
            None => {
                *reloader = Some(HotReloader::start(self)?);
            }
        }
        Ok(())
    }

    /// Stops the hot-reloading.
    ///
    /// If [`hot_reload`] has not been called on this `AssetCache`, this method
    /// has no effect. Hot-reloading will restart when [`hot_reload`] is called
    /// again.
    ///
    /// [`hot_reload`]: #method.hot_reload
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    pub fn stop_hot_reloading(&self) {
        let mut reloader = self.reloader.lock();
        reloader.take();
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn reload(&self, id: &str, ext: &str, content: Vec<u8>) {
        let range = AccessKey::range_of(id);
        let cache = self.assets.read();

        for (k, v) in cache.range(range) {
            let type_id = k.type_id.unwrap();

            if type_id.ext == ext {
                unsafe {
                    if let Err(err) = (type_id.reload)(v, content.clone()) {
                        log::warn!("Cannot reload {:?}: {}", id, err);
                    }
                }
            }
        }
    }
}

#[cfg(feature = "hot-reloading")]
unsafe fn reload_one<A: Asset>(entry: &CacheEntry, content: Vec<u8>) -> Result<(), AssetError> {
    let asset = A::load_from_raw(content)?;
    entry.write(asset);
    Ok(())
}

impl fmt::Debug for AssetCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("path", &self.path)
            .field("assets", &self.assets.read())
            .finish()
    }
}
