//! Conveniently load, store and cache external resources.
//! 
//! 
//! It has multiple goals
//! - Easy to use: Rusty API
//! - Light: Pay for what you take, no dependencies bloat
//! - Fast: Share your resources between threads without using expensive `Arc::clone`
//!
//! ## Cargo features
//! 
//! No features are enabled by default.
//!
//! ### Additionnal loaders
//! - `bincode`: Bincode deserialization
//! - `cbor`: CBOR deserialization
//! - `json`: JSON deserialization
//! - `ron`: RON deserialization
//! - `toml`: TOML deserialization
//! - `yaml`: YAML deserialization
//! 
//! ### Internal features
//!
//! These features change inner data structures implementations.
//!
//! - `hashbrown`: Use *hashbrown* crate's HashMap
//! - `parking_lot`: Use *parking_lot* crate's synchronisation primitives
//!
//! ## Example
//!
//! If the file `assets/common/test.ron` contains this:
//!
//! ```text
//! Point(
//!     x: 5,
//!     y: -6,
//! )
//! ```
//!
//! Then you can load it this way (with feature `ron` enabled):
//!
//! ```
//! # cfg_if::cfg_if! { if #[cfg(feature = "ron")] {
//! use assets_manager::{Asset, AssetCache, loader};
//! use serde::Deserialize;
//!
//! // The struct you want to load
//! #[derive(Deserialize)]
//! struct Point {
//!     x: i32,
//!     y: i32,
//! }
//!
//! // Specify how you want the structure to be loaded
//! impl Asset for Point {
//!     // The extension of the files to look into
//!     const EXT: &'static str = "ron";
//!
//!     // The serialization format
//!     type Loader = loader::RonLoader;
//! }
//!
//!
//! // Create a new cache to load assets under the "./assets" folder
//! let cache = AssetCache::new("assets");
//!
//! // Get a lock on the asset
//! let asset_lock = cache.load::<Point>("common.test")?;
//!
//! // Lock the asset for reading
//! // Any number of read locks can exist at the same time,
//! // but none can exist when the asset is reloaded
//! let point = asset_lock.read();
//!
//! // The asset is now ready to be used
//! assert_eq!(point.x, 5);
//! assert_eq!(point.y, -6);
//!
//! // Loading the same asset retreives it from the cache
//! let other_lock = cache.load("common.test")?;
//! assert!(asset_lock.ptr_eq(&other_lock));
//!
//! # }}
//! # Ok::<(), assets_manager::AssetError>(())
//! ```

#![doc(html_root_url = "https://docs.rs/assets_manager/0.1")]

#![warn(
    missing_docs,
    missing_debug_implementations,
)]

pub mod loader;
#[doc(inline)]
pub use loader::Loader;

mod lock;
use lock::{RwLock, rwlock, CacheEntry};
#[doc(inline)]
pub use lock::{AssetRefLock, AssetRef};

mod error;
#[doc(inline)]
pub use error::AssetError;

#[cfg(test)]
mod tests;

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

/// Used to cache assets.
///
/// It uses interior mutability, so assets can be added in the cache without
/// requiring a mutable reference, but one is required to remove an asset.
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
    /// Note: this function requires a write lock on the asset, and will block
    /// until one is aquired, ie no read lock can exist at the same time.
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


/// An asset is a type loadable from a file.
///
/// `Asset`s can loaded and retreived by an [`AssetCache`].
///
/// [`AssetCache`]: struct.AssetCache.html
pub trait Asset: Sized + Send + Sync + 'static {
    /// The extension used by the asset files from the given asset type.
    ///
    /// Use `""` for no extension.
    const EXT: &'static str;

    /// Specifies a way to to convert raw bytes into the asset.
    ///
    /// See module [`loader`] for implementations of common conversions.
    ///
    /// [`loader`]: loader/index.html
    type Loader: Loader<Self>;

    /// Create an asset value from raw parts.
    ///
    /// This function is not meant to be used directly, but rather to
    /// be overriden if you don't want or need to implement [`Loader`].
    /// In that case, you should use [`CustomLoader`] as [`Loader`]
    ///
    /// [`Loader`]: loader/trait.Loader.html
    /// [`CustomLoader`]: loader/struct.CustomLoader.html
    #[inline]
    fn load_from_raw(content: Vec<u8>) -> Result<Self, AssetError> {
        Self::Loader::load(content).map_err(|e| AssetError::LoadError(e))
    }
}
