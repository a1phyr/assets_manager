use crate::{
    Asset,
    AssetError,
    lock::{RwLock, rwlock, CacheEntry, AssetRefLock},
};

use std::{
    any::TypeId,
    borrow::Borrow,
    fmt,
    fs,
    path::PathBuf,
};

#[cfg(feature = "hashbrown")]
use hashbrown::HashMap;
#[cfg(not(feature = "hashbrown"))]
use std::collections::HashMap;

#[derive(Debug, Hash, PartialEq, Eq)]
#[repr(C)]
struct Key {
    id: Box<str>,
    type_id: TypeId,
}

impl Key {
    #[inline]
    fn new<T: 'static>(id: Box<str>) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
        }
    }
}

impl<'a> AccessKey<'a> {
    #[inline]
    fn new<T: 'static>(id: &'a str) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq)]
#[repr(C)]
struct AccessKey<'a> {
    id: &'a str,
    type_id: TypeId,
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
/// let cache = AssetCache::new("assets");
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
/// // Drop the guard to avoid a deadlock
/// drop(point);
///
/// // Reload the asset from the filesystem
/// cache.reload::<Point>("common.position")?;
/// println!("New position: {:?}", point_lock.read());
///
/// # }}
/// # Ok::<(), assets_manager::AssetError>(())
/// ```
pub struct AssetCache<'a> {
    assets: RwLock<HashMap<Key, CacheEntry>>,
    path: &'a str,
}

impl<'a> AssetCache<'a> {
    /// Creates a new cache.
    ///
    /// Assets will be searched in the directory `path`
    #[inline]
    pub fn new(path: &str) -> AssetCache {
        AssetCache {
            assets: RwLock::new(HashMap::new()),
            path,
        }
    }

    pub(crate) fn add_asset<A: Asset>(&self, id: String, asset: A) -> AssetRefLock<A> {
        let entry = CacheEntry::new(asset);
        // Safety:
        // We just created the asset with the good type
        // The cache entry is garantied to live long enough
        let asset = unsafe { entry.get_ref() };

        let key = Key::new::<A>(id.into());
        let mut cache = rwlock::write(&self.assets);
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

        let asset = self.load_from_path(id)?;
        Ok(self.add_asset(id.to_string(), asset))
    }

    /// Loads an asset from the cache.
    ///
    /// This function does not attempt to load the asset from the filesystem if
    /// it is not found in the cache.
    pub fn load_cached<A: Asset>(&self, id: &str) -> Option<AssetRefLock<A>> {
        let key = AccessKey::new::<A>(id);
        let cache = rwlock::read(&self.assets);
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
    /// means that you MUST NOT call this method if you have an `AssetRef` on
    /// the same asset, or it may cause a deadlock.
    ///
    /// # Errors
    ///
    /// Error cases are the same as [`load`].
    ///
    /// If an error occurs, the asset is left unmodified.
    ///
    /// [`load`]: fn.load.html
    pub fn reload<A: Asset>(&self, id: &str) -> Result<AssetRefLock<A>, AssetError> {
        let asset = self.load_from_path(id)?;

        let cache = rwlock::read(&self.assets);
        if let Some(cached) = cache.get(&AccessKey::new::<A>(id)) {
            return unsafe { Ok(cached.write(asset)) };
        }
        drop(cache);

        Ok(self.add_asset(id.to_string(), asset))
    }


    fn load_from_path<A: Asset>(&self, id: &str) -> Result<A, AssetError> {
        let mut path = PathBuf::from(self.path);
        path.push(id.replace(".", "/"));
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
        let cache = rwlock::get_mut(&mut self.assets);
        cache.remove(&key);
    }

    /// Take ownership on an asset.
    ///
    /// The corresponding asset is removed from the cache.
    pub fn take<A: Asset>(&mut self, id: &str) -> Option<A> {
        let key = AccessKey::new::<A>(id);
        let cache = rwlock::get_mut(&mut self.assets);
        cache.remove(&key).map(|entry| unsafe { entry.into_inner() })
    }

    /// Clears the cache.
    #[inline]
    pub fn clear(&mut self) {
        rwlock::get_mut(&mut self.assets).clear();
    }
}

impl fmt::Debug for AssetCache<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("path", &self.path)
            .field("assets", &self.assets.read())
            .finish()
    }
}
