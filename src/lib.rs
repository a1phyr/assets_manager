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
//! - `hot-reloading`: Add hot-reloading
//!
//! ### Additionnal loaders
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
//! These features change inner data structures implementations.
//!
//! - `parking_lot`: Use *parking_lot* crate's synchronisation primitives
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
//!     const EXT: &'static str = "ron";
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

#![doc(html_root_url = "https://docs.rs/assets_manager/0.1")]

#![warn(
    missing_docs,
    missing_debug_implementations,
)]

mod cache;
#[doc(inline)]
pub use cache::AssetCache;

pub mod loader;

mod lock;
#[doc(inline)]
pub use lock::{AssetRefLock, AssetRef};

mod error;
#[doc(inline)]
pub use error::AssetError;

#[cfg(feature = "hot-reloading")]
mod hot_reloading;

#[cfg(test)]
mod tests;


/// An asset is a type loadable from a file.
///
/// `Asset`s can loaded and retreived by an [`AssetCache`].
///
/// ## Example
///
/// Suppose you make a physics simulutation, and you positions and speeds in
/// a Bincode-encoded files, with extension ".data"
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
///     const EXT: &'static str = "data";
///     type Loader = loader::BincodeLoader;
/// }
/// # }}
/// ```
/// [`AssetCache`]: struct.AssetCache.html
pub trait Asset: Sized + Send + Sync + 'static {
    /// The extension used by the asset files from the given asset type.
    ///
    /// It must not contain the `.` caracter.
    ///
    /// Use `""` for no extension.
    const EXT: &'static str;

    /// Specifies a way to to convert raw bytes into the asset.
    ///
    /// See module [`loader`] for implementations of common conversions.
    ///
    /// [`loader`]: loader/index.html
    type Loader: loader::Loader<Self>;
}
