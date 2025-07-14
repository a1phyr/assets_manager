//! Conveniently load, store and cache external resources.
//!
//! This crate aims at providing a filesystem abstraction to easily load external resources.
//! It was originally thought for games, but can, of course, be used in other contexts.
//!
//! The structure [`AssetCache`] is the entry point of the crate. See [`Asset`] documentation
//! to see how to define custom asset types.
//!
//! # Cargo features
//!
//! - `hot-reloading`: Add hot-reloading.
//! - `macros`: Add support for deriving `Asset` trait.
//!
//! ### Additional sources
//!
//! Enable reading assets from sources other than the filesystem.
//! These sources are defined in the [`source`] module:
//!
//! - `embedded`: Embeds asset files directly in your binary at compile time
//! - `zip`: Reads assets from ZIP archives
//!   - Optional compression: `zip-deflate`, `zip-zstd`
//! - `tar`: Reads assets from TAR archives
//!
//! ### Additional formats
//!
//! These features add support for various asset formats:
//!
//! - Serialisation formats (using [`serde`]): `bincode`, `json`,
//!   `msgpack`, `ron`, `toml`, `yaml`.
//! - Image formats (using [`image`]): `bmp`, `jpeg`, `png` `webp`.
//! - GlTF format (using [`gltf`]): `gltf`.
//!
//! ## External crates support
//!
//! Support of some other crates is done in external crates:
//! - [`ggez`](https://github.com/ggez/ggez): [`ggez-assets_manager`](https://crates.io/crates/ggez-assets_manager)
//! - [`kira`](https://github.com/tesselode/kira): [`assets_manager-kira`](https://crates.io/crates/assets_manager-kira)
//! - [`rodio`](https://github.com/RustAudio/rodio): [`assets_manager-rodio`](https://crates.io/crates/assets_manager-rodio)
//!
//! ### Internal features
//!
//! These features change internal data structures implementations.
//!
//! - [`parking_lot`]: Use `parking_lot`'s synchronization primitives.
//! - `faster-hash`: Use a faster hashing algorithm (enabled by default).
//!
//! # Basic example
//!
//! Given a file `assets/common/position.ron` which contains this:
//!
//! ```text
//! Point(
//!     x: 5,
//!     y: -6,
//! )
//! ```
//!
//! You can load and use it as follows:
//!
//! ```
//! # cfg_if::cfg_if! { if #[cfg(feature = "ron")] {
//! use assets_manager::{BoxedError, AssetCache, FileAsset};
//! use serde::Deserialize;
//! use std::borrow::Cow;
//!
//! // The struct you want to load
//! #[derive(Deserialize)]
//! struct Point {
//!     x: i32,
//!     y: i32,
//! }
//!
//! // Specify how you want the structure to be loaded
//! impl FileAsset for Point {
//!     // The extension of the files to look into
//!     const EXTENSION: &'static str = "ron";
//!
//!     // The serialization format
//!     //
//!     // In this specific case, the derive macro could be used but we use the
//!     // full version as a demo.
//!     fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Result<Self, BoxedError> {
//!         assets_manager::asset::load_ron(&bytes)
//!     }
//! }
//!
//! // Create a new cache to load assets under the "./assets" folder
//! let cache = AssetCache::new("assets")?;
//!
//! // Get a handle on the asset
//! // This will load the file `./assets/common/position.ron`
//! let handle = cache.load::<Point>("common.position")?;
//!
//! // Lock the asset for reading
//! // Any number of read locks can exist at the same time,
//! // but none can exist when the asset is reloaded
//! let point = handle.read();
//!
//! // The asset is now ready to be used
//! assert_eq!(point.x, 5);
//! assert_eq!(point.y, -6);
//!
//! // Loading the same asset retreives it from the cache
//! let other_handle = cache.load("common.position")?;
//! assert!(std::ptr::eq(handle, other_handle));
//! # }}
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Hot-reloading
//!
//! Hot-reloading is a major feature of `assets_manager`: when a file is added,
//! modified or deleted, the values of all assets that depend on this file are
//! automatically and transparently updated. It is managed automatically in the
//! background.
//!
//! See the [`asset`] module for a precise description of how assets interact
//! with hot-reloading.

#![warn(missing_docs, missing_debug_implementations)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate self as assets_manager;

pub mod asset;
#[allow(deprecated)]
pub use asset::{Asset, Compound, FileAsset, Storable};

mod cache;
pub use cache::AssetCache;

mod dirs;
pub use dirs::{Directory, RawDirectory, RawRecursiveDirectory, RecursiveDirectory};

mod error;
pub use error::{BoxedError, Error};

mod map;

mod entry;
pub use entry::{
    ArcHandle, ArcUntypedHandle, AssetReadGuard, AtomicReloadId, Handle, ReloadId, ReloadWatcher,
    UntypedHandle, WeakHandle, WeakUntypedHandle,
};

mod key;

pub mod source;

#[cfg_attr(not(feature = "hot-reloading"), path = "hot_reloading/disabled.rs")]
pub mod hot_reloading;

mod utils;
#[cfg(feature = "utils")]
#[cfg_attr(docsrs, doc(cfg(feature = "utils")))]
pub use utils::cell::OnceInitCell;
pub use utils::{SharedBytes, SharedString};

/// Implements [`Asset`] for a type.
///
/// Note that the type must implement the right traits for it to work (eg
/// `serde::Deserialize` or `std::str::FromStr`).
///
/// # Supported formats
///
/// - `"json"`: Use [`asset::load_json`] and extension `.json`
/// - `"ron"`: Use [`asset::load_ron`] and extension `.ron`
/// - `"toml"`: Use [`asset::load_toml`] and extension `.toml`
/// - `"txt"`: Use [`asset::load_text`] and extension `.txt`
/// - `"yaml"` or `"yml"`: Use [`asset::load_yaml`] and extensions `.yaml` and `.yml`
///
/// # Example
///
/// ```rust
/// # cfg_if::cfg_if! { if #[cfg(feature = "ron")] {
/// use assets_manager::{Asset, AssetCache, BoxedError};
/// // Define a type loaded as ron
/// #[derive(Asset, serde::Deserialize)]
/// #[asset_format = "ron"]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
///
/// // Define a type loaded as text
/// #[derive(Asset)]
/// #[asset_format = "txt"]
/// struct Name(String);
///
/// impl std::str::FromStr for Name {
///     type Err = BoxedError;
///
///     fn from_str(s: &str) -> Result<Self, BoxedError> {
///         Ok(Self(String::from(s)))
///     }
/// }
///
/// let cache = AssetCache::new("assets")?;
///
/// // Load "assets/common/position.ron"
/// let position = cache.load::<Point>("common.position")?;
/// assert_eq!(position.read().x, 5);
/// assert_eq!(position.read().y, -6);
///
/// // Load "assets/common/name.txt"
/// let name = cache.load::<Name>("common.name")?;
/// assert_eq!(name.read().0, "Aragorn");
/// # }}
/// # Ok::<(), assets_manager::BoxedError>(())
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
#[cfg(feature = "macros")]
pub use assets_manager_macros::Asset;

#[deprecated = "Use `AssetCache` instead"]
/// Type alias to `AssetCache` to ease migration.
pub type AnyCache<'a> = &'a AssetCache;

#[cfg(test)]
mod tests;
