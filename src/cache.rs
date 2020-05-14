//! Definition of the cache
use crate::{
    Asset,
    AssetError,
    dirs::{CachedDir, DirReader},
    loader::Loader,
    lock::{RwLock, CacheEntry, AssetRefLock},
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

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn new_with(id: &'a str, type_id: TypeId) -> Self {
        Self { id, type_id }
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
/// **Note**: This cache uses paths of files to refer to them, so using symbolic or
/// hard links can lead to suprising behaviour (especially with hot-reloading), and
/// thus should be avoided
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
    path: PathBuf,

    pub(crate) assets: RwLock<HashMap<Key, CacheEntry, RandomState>>,
    dirs: RwLock<HashMap<Key, CachedDir, RandomState>>,

    #[cfg(feature = "hot-reloading")]
    reloader: Mutex<Option<HotReloader>>,
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
    /// An error will be returned if `path` is not valid readable directory.
    #[inline]
    pub fn new<P: AsRef<Path>>(path: P) -> Result<AssetCache, io::Error> {
        let path = path.as_ref().canonicalize()?;
        let _ = path.read_dir()?;

        Ok(AssetCache {
            assets: RwLock::new(HashMap::with_hasher(RandomState::new())),
            dirs: RwLock::new(HashMap::with_hasher(RandomState::new())),
            path,

            #[cfg(feature = "hot-reloading")]
            reloader: Mutex::new(None),
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

    pub(crate) fn path_of(&self, id: &str, ext: &str) -> PathBuf {
        let mut path = self.path.clone();
        path.extend(id.split('.'));
        path.set_extension(ext);
        path
    }

    /// Adds an asset to the cache
    pub(crate) fn add_asset<A: Asset>(&self, id: String) -> Result<AssetRefLock<A>, AssetError> {
        let path = self.path_of(&id, A::EXT);
        let asset: A = self.load_from_fs(&path)?;

        let entry = CacheEntry::new(asset);
        // Safety:
        // We just created the asset with the good type
        // The cache entry is garantied to live long enough
        let asset = unsafe { entry.get_ref() };

        #[cfg(feature = "hot-reloading")]
        {
            let mut watched = self.watched.lock();
            watched.add::<A>(path, id.clone());
        }

        let key = Key::new::<A>(id.into());
        let mut cache = self.assets.write();
        cache.insert(key, entry);

        Ok(asset)
    }

    fn add_dir<A: Asset>(&self, id: String) -> Result<DirReader<A>, io::Error> {
        let dir = CachedDir::load::<A>(self, &id)?;
        let reader = unsafe { dir.read(self) };

        let key = Key::new::<A>(id.into());
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
    pub fn load<A: Asset>(&self, id: &str) -> Result<AssetRefLock<A>, AssetError> {
        match self.load_cached(id) {
            Some(asset) => Ok(asset),
            None => self.add_asset(id.to_string()),
        }
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
        let cache = self.assets.read();
        if let Some(cached) = cache.get(&AccessKey::new::<A>(id)) {
            let path = self.path_of(id, A::EXT);
            let asset = self.load_from_fs(&path)?;
            return unsafe { Ok(cached.write(asset)) };
        }
        drop(cache);

        self.add_asset(id.to_string())
    }

    fn load_from_fs<A: Asset>(&self, path: &Path) -> Result<A, AssetError> {
        let content = fs::read(&path)?;
        A::Loader::load(content).map_err(|e| AssetError::LoadError(e))
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

        self.add_dir(id.to_string())
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
}

impl fmt::Debug for AssetCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("path", &self.path)
            .field("assets", &self.assets.read())
            .finish()
    }
}
