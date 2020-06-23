//! Definition of the cache
use crate::{
    Asset,
    AssetError,
    dirs::{CachedDir, DirReader},
    loader::Loader,
    lock::{RwLock, CacheEntry, AssetRef},
};

#[cfg(feature = "hot-reloading")]
use crate::{
    lock::Mutex,
    hot_reloading::{HotReloader, WatchedPaths}
};

use std::{
    any::TypeId,
    borrow::Borrow,
    collections::HashMap,
    error::Error,
    fmt,
    fs,
    io,
    path::{Path, PathBuf},
};

use crate::RandomState;


/// The key used to identify assets
///
/// **Note**: This definition has to kept in sync with [`AccessKey`]'s one.
///
/// [`AccessKey`]: struct.AccessKey.html
#[derive(PartialEq, Eq, Hash)]
#[repr(C)]
pub(crate) struct Key {
    id: Box<str>,
    type_id: TypeId,
}

impl Key {
    /// Creates a Key with the given type and id.
    #[inline]
    fn new<T: Asset>(id: Box<str>) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
        }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn new_with(id: Box<str>, type_id: TypeId) -> Self {
        Self { id, type_id }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// A borrowed version of [`Key`]
///
/// [`Key`]: struct.Key.html
#[derive(PartialEq, Eq, Hash)]
#[repr(C)]
pub(crate) struct AccessKey<'a> {
    id: &'a str,
    type_id: TypeId,
}

impl<'a> AccessKey<'a> {
    /// Creates an AccessKey for the given type and id.
    #[inline]
    fn new<T: Asset>(id: &'a str) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
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
            .field("type_id", &self.type_id)
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
/// the extension. Given that, you cannot use `.` in your file names except for
/// the extension.
///
/// **Note**: Using symbolic or hard links within the cached directory can lead
/// to surprising behaviour (especially with hot-reloading), and thus should be
/// avoided.
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
///     const EXTENSION: &'static str = "ron";
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
    path: PathBuf,

    pub(crate) assets: RwLock<HashMap<Key, CacheEntry, RandomState>>,
    pub(crate) dirs: RwLock<HashMap<Key, CachedDir, RandomState>>,

    #[cfg(feature = "hot-reloading")]
    reloader: Mutex<HotReloader>,
    #[cfg(feature = "hot-reloading")]
    pub(crate) watched: Mutex<WatchedPaths>,
}

impl AssetCache {
    /// Creates a new cache.
    ///
    /// Assets will be searched in the directory given by `path`. Symbolic links
    /// will be followed.
    ///
    /// # Errors
    ///
    /// An error will be returned if `path` is not valid readable directory or
    /// if hot-reloading failed to start (if feature `hot-reloading` is used).
    #[inline]
    pub fn new<P: AsRef<Path>>(path: P) -> Result<AssetCache, CacheError> {
        let path = path.as_ref().canonicalize().map_err(ErrorKind::Io)?;
        let _ = path.read_dir().map_err(ErrorKind::Io)?;

        #[cfg(feature = "hot-reloading")]
        let reloader = Mutex::new(HotReloader::start(&path).map_err(ErrorKind::Notify)?);

        Ok(AssetCache {

            assets: RwLock::new(HashMap::with_hasher(RandomState::new())),
            dirs: RwLock::new(HashMap::with_hasher(RandomState::new())),
            path,

            #[cfg(feature = "hot-reloading")]
            reloader,
            #[cfg(feature = "hot-reloading")]
            watched: Mutex::new(WatchedPaths::new()),
        })
    }

    /// Gets the path of the cache's root.
    ///
    /// The path is currently given as absolute, but this may change in the future.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn path_of(&self, id: &str) -> PathBuf {
        let mut path = self.path.clone();
        path.extend(id.split('.'));
        path
    }

    /// Adds an asset to the cache
    pub(crate) fn add_asset<A: Asset>(&self, id: Box<str>) -> Result<AssetRef<A>, AssetError<A>> {
        let path = self.path_of(&id);

        #[cfg(feature = "hot-reloading")]
        for ext in A::EXTENSIONS {
            let mut path = path.clone();
            path.set_extension(ext);
            self.watched.lock().add_file::<A>(path, id.clone());
        }

        let asset: A = load_from_fs(path)?;

        let entry = CacheEntry::new(asset);
        // Safety:
        // We just created the asset with the good type
        // The cache entry is garantied to live long enough
        let asset = unsafe { entry.get_ref() };

        let key = Key::new::<A>(id);
        let mut cache = self.assets.write();
        cache.insert(key, entry);

        Ok(asset)
    }

    fn add_dir<A: Asset>(&self, id: Box<str>) -> Result<DirReader<A>, io::Error> {
        let path = self.path_of(&id);
        let dir = CachedDir::load::<A>(self, &path, &id)?;
        let reader = unsafe { dir.read(self) };

        #[cfg(feature = "hot-reloading")]
        self.watched.lock().add_dir::<A>(path, id.clone());

        let key = Key::new::<A>(id);
        let mut dirs = self.dirs.write();
        dirs.insert(key, dir);

        Ok(reader)
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
    pub fn load<A: Asset>(&self, id: &str) -> Result<AssetRef<A>, AssetError<A>> {
        match self.load_cached(id) {
            Some(asset) => Ok(asset),
            None => self.add_asset(id.into()),
        }
    }

    /// Loads an asset from the cache.
    ///
    /// This function does not attempt to load the asset from the filesystem if
    /// it is not found in the cache.
    pub fn load_cached<A: Asset>(&self, id: &str) -> Option<AssetRef<A>> {
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
    pub fn load_expect<A: Asset>(&self, id: &str) -> AssetRef<A> {
        self.load(id).unwrap_or_else(|err| {
            panic!("Failed to load essential asset {:?}: {}", id, err)
        })
    }

    /// Reloads an asset from the filesystem.
    ///
    /// It does not matter whether the asset has been loaded yet.
    ///
    /// **Note**: this function requires a write lock on the asset, and will block
    /// until one is aquired, ie no read lock can exist at the same time. This
    /// means that you **must not** call this method if you have an `AssetGuard`
    /// on the same asset, or it may cause a deadlock.
    ///
    /// # Errors
    ///
    /// Error cases are the same as [`load`].
    ///
    /// If an error occurs, the asset is left unmodified.
    ///
    /// [`load`]: fn.load.html
    pub fn force_reload<A: Asset>(&self, id: &str) -> Result<AssetRef<A>, AssetError<A>> {
        let cache = self.assets.read();
        if let Some(cached) = cache.get(&AccessKey::new::<A>(id)) {
            let path = self.path_of(id);
            let asset = load_from_fs(path)?;
            return unsafe { Ok(cached.write(asset)) };
        }
        drop(cache);

        self.add_asset(id.into())
    }

    /// Load all assets of a given type in a directory.
    ///
    /// The directory's id is constructed the same way as assets. To specify
    /// the cache's root, give the empty string (`""`) as id.
    ///
    /// The returned structure can be iterated on to get the loaded assets.
    ///
    /// # Error
    ///
    /// An error is returned if the given id does not match a valid readable
    /// directory.
    pub fn load_dir<A: Asset>(&self, id: &str) -> io::Result<DirReader<A>> {
        let dirs = self.dirs.read();
        if let Some(dir) = dirs.get(&AccessKey::new::<A>(id)) {
            return unsafe { Ok(dir.read(self)) };
        }
        drop(dirs);

        self.add_dir(id.into())
    }

    /// Load an owned version of the asset
    ///
    /// Note that it will not try to fetch it from the cache nor to cache it.
    /// In addition, hot-reloading will not affect the returned value.
    pub fn load_owned<A: Asset>(&self, id: &str) -> Result<A, AssetError<A>> {
        let path = self.path_of(id);
        load_from_fs(path)
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
        self.dirs.get_mut().clear();

        #[cfg(feature = "hot-reloading")]
        self.watched.get_mut().clear();
    }

    /// Reloads changed assets.
    ///
    /// This function is typically called within a loop.
    ///
    /// If an error occurs while reloading an asset, a warning will be logged
    /// and the asset will be left unchanged.
    ///
    /// This function blocks the current thread until all changed assets are
    /// reloaded, but it does not perform any I/O. However, it needs to lock
    /// some assets for writing, so you **must not** have any [`AssetGuard`]
    /// from the given `AssetCache`, or you might experience deadlocks. You are
    /// free to keep [`AssetRef`]s, though. The same restriction applies to
    /// [`ReadDir`] and [`ReadAllDir`].
    ///
    /// [`AssetGuard`]: struct.AssetGuard.html
    /// [`AssetRef`]: struct.AssetRef.html
    /// [`ReadDir`]: struct.ReadDir.html
    /// [`ReadAllDir`]: struct.ReadAllDir.html
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    pub fn hot_reload(&self) {
        self.reloader.lock().reload(self);
    }
}

impl fmt::Debug for AssetCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("path", &self.path)
            .field("assets", &self.assets.read())
            .field("dirs", &self.dirs.read())
            .finish()
    }
}

fn load_from_fs<A: Asset>(mut path: PathBuf) -> Result<A, AssetError<A>> {
    // Compile-time assert that the asset type has at least one extension
    let _ = <A as Asset>::_AT_LEAST_ONE_EXTENSION_REQUIRED;

    let mut err = None;

    for ext in A::EXTENSIONS {
        path.set_extension(ext);
        let content = fs::read(&path).map(Into::into);

        match A::Loader::load(content, ext) {
            Err(e) => err = Some(e),
            asset => return asset,
        }
    }

    // The for loop is taken at least once, so unwrap never panics
    Err(err.unwrap())
}


enum ErrorKind {
    Io(io::Error),
    #[cfg(feature = "hot-reloading")]
    Notify(notify::Error),
}

/// An error which occurs when creating a cache.
///
/// This error can be returned by [`AssetCache::new`].
///
/// [`AssetCache::new`]: struct.AssetCache.html#method.new
pub struct CacheError(ErrorKind);

impl From<ErrorKind> for CacheError {
    fn from(err: ErrorKind) -> Self {
        Self(err)
    }
}

impl fmt::Debug for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_tuple("CacheError");

        match &self.0 {
            ErrorKind::Io(err) => debug.field(err),
            #[cfg(feature = "hot-reloading")]
            ErrorKind::Notify(err) => debug.field(err),
        }.finish()
    }
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::Io(err) => err.fmt(f),
            #[cfg(feature = "hot-reloading")]
            ErrorKind::Notify(err) => err.fmt(f),
        }
    }
}

impl Error for CacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.0 {
            ErrorKind::Io(err) => Some(err),
            #[cfg(feature = "hot-reloading")]
            ErrorKind::Notify(err) => Some(err),
        }
    }
}
