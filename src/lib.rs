//! Conveniently load, store and cache external resources.
//!
//! This crate aims at providing a filesystem abstraction to easily load external resources.
//! It was originally thought for games, but can of course be used in other contexts.
//!
//! The structure [`AssetCache`] is the entry point of the crate.
//!
//! [`AssetCache`]: struct.AssetCache.html
//!
//! ## Cargo features
//!
//! - `hot-reloading`: Add hot-reloading
//! - `embedded`: Add embedded source
//!
//! ### Additionnal loaders
//!
//! - `bincode`: Bincode deserialization
//! - `cbor`: CBOR deserialization
//! - `json`: JSON deserialization
//! - `msgpack`: MessagePack deserialization
//! - `ron`: RON deserialization
//! - `toml`: TOML deserialization
//! - `yaml`: YAML deserialization
//!
//! ### Internal features
//!
//! These features change inner data structures implementations. They usually
//! increase performances, and are therefore enabled by default.
//!
//! - `parking_lot`: Use *parking_lot* crate's synchronisation primitives
//! - `ahash`: Use ahash algorithm instead Sip1-3 used in `std`.
//!
//! ## Example
//!
//! If the file `assets/common/position.ron` contains this:
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
//!     const EXTENSION: &'static str = "ron";
//!
//!     // The serialization format
//!     type Loader = loader::RonLoader;
//! }
//!
//!
//! // Create a new cache to load assets under the "./assets" folder
//! let cache = AssetCache::new("assets")?;
//!
//! // Get a lock on the asset
//! let asset_lock = cache.load::<Point>("common.position")?;
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
//! let other_lock = cache.load("common.position")?;
//! assert!(asset_lock.ptr_eq(&other_lock));
//!
//! # }}
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![doc(html_root_url = "https://docs.rs/assets_manager/0.3.2")]

#![warn(
    missing_docs,
    missing_debug_implementations,
)]

#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate self as assets_manager;

mod cache;
pub use cache::AssetCache;

mod dirs;
pub use dirs::{DirReader, ReadAllDir, ReadDir};

mod error;
pub use error::{BoxedError, Error};

pub mod loader;

mod entry;
pub use entry::{AssetHandle, AssetGuard};

pub mod source;

#[cfg(feature = "hot-reloading")]
mod hot_reloading;

mod utils;

#[cfg(test)]
mod tests;

use std::sync::Arc;


/// An asset is a type loadable from a file.
///
/// `Asset`s can loaded and retreived by an [`AssetCache`].
///
/// # Extension
///
/// You can provide several extensions that will be used to search and load
/// assets. When loaded, each extension is tried in order until a file is
/// correctly loaded or no extension remain. The empty string `""` means a file
/// without extension. You cannot use character `.`.
///
/// The `EXTENSION` field is a convenient shortcut if your asset uses only one
/// extension. If you set a value for `EXTENSIONS` too, this field is ignored.
///
/// If neither `EXTENSION` nor `EXTENSIONS` is set, the default is no extension.
///
/// If you use hot-reloading, the asset will be reloaded each time one of the
/// file with the given extension is touched.
///
/// # Example
///
/// Suppose you make a physics simulutation, and you store positions and speeds
/// in a Bincode-encoded files, with extension ".data".
///
/// ```no_run
/// # cfg_if::cfg_if! { if #[cfg(feature = "bincode")] {
/// use assets_manager::{Asset, loader};
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Vector {
///     x: f32,
///     y: f32,
///     z: f32,
/// }
///
/// #[derive(Deserialize)]
/// struct World {
///     pos: Vec<Vector>,
///     speed: Vec<Vector>,
/// }
///
/// impl Asset for World {
///     const EXTENSION: &'static str = "data";
///     type Loader = loader::BincodeLoader;
/// }
/// # }}
/// ```
/// [`AssetCache`]: struct.AssetCache.html
pub trait Asset: Sized + Send + Sync + 'static {
    /// Use this field if your asset only uses one extension.
    ///
    /// This value is ignored if you set `EXTENSIONS` too.
    const EXTENSION: &'static str = "";

    /// This field enables you to specify multiple extension for an asset.
    ///
    /// If `EXTENSION` is provided, you don't have to set this constant.
    ///
    /// If this array is empty, loading an asset of this type returns
    /// [`Error::NoDefaultValue`] unless a default value is provided with the
    /// `default_value` method.
    ///
    /// [`Error::NoDefaultValue`]: enum.Error.html#variant.NoDefaultValue
    const EXTENSIONS: &'static [&'static str] = &[Self::EXTENSION];

    /// Specifies a way to to convert raw bytes into the asset.
    ///
    /// See module [`loader`] for implementations of common conversions.
    ///
    /// [`loader`]: loader/index.html
    type Loader: loader::Loader<Self>;

    /// Specifies a eventual default value to use if an asset fails to load. If
    /// this method returns `Ok`, the returned value is used as an asset. In
    /// particular, if this method always returns `Ok`, all `AssetCache::load*`
    /// (except `load_cached`) are guarantied not to fail.
    ///
    /// The `id` parameter is given to easily report the error.
    ///
    /// By default, this method always returns an error.
    ///
    /// # Example
    ///
    /// On error, log it and return a default value:
    ///
    /// ```no_run
    /// # cfg_if::cfg_if! { if #[cfg(feature = "json")] {
    /// use assets_manager::{Asset, Error, loader};
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, Default)]
    /// struct Item {
    ///     name: String,
    ///     kind: String,
    /// }
    ///
    /// impl Asset for Item {
    ///     const EXTENSION: &'static str = "json";
    ///     type Loader = loader::JsonLoader;
    ///
    ///     fn default_value(id: &str, error: Error) -> Result<Item, Error> {
    ///         eprintln!("Error loading {}: {}. Using default value", id, error);
    ///         Ok(Item::default())
    ///     }
    /// }
    /// # }}
    /// ```
    #[inline]
    #[allow(unused_variables)]
    fn default_value(id: &str, error: Error) -> Result<Self, Error> {
        Err(error)
    }
}

impl<A> Asset for Box<A>
where
    A: Asset,
{
    const EXTENSIONS: &'static [&'static str] = A::EXTENSIONS;
    type Loader = loader::LoadFromAsset<A>;

    #[inline]
    fn default_value(id: &str, error: Error) -> Result<Box<A>, Error> {
        A::default_value(id, error).map(Box::new)
    }
}

impl<A> Asset for Arc<A>
where
    A: Asset,
{
    const EXTENSIONS: &'static [&'static str] = A::EXTENSIONS;
    type Loader = loader::LoadFromAsset<A>;

    #[inline]
    fn default_value(id: &str, error: Error) -> Result<Arc<A>, Error> {
        A::default_value(id, error).map(Arc::new)
    }
}
