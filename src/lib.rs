//! Conveniently load, store and cache external resources.
//!
//! This crate aims at providing a filesystem abstraction to easily load external resources.
//! It was originally thought for games, but can, of course, be used in other contexts.
//!
//! The structure [`AssetCache`] is the entry point of the crate.
//!
//! # Cargo features
//!
//! - `hot-reloading`: Add hot-reloading.
//! - `macros`: Add support for deriving `Asset` trait.
//!
//! ### Additional sources
//!
//! These features enable to read assets from other sources than the file
//! system. They are defined in the [`source`] module.
//!
//! - `embedded`: Embed assets files directly in your binary.
//! - `zip`: Read asset from zip archives. Note that no decompression method is
//!   enabled by default, but you can do so with the following features:
//!   - `zip-bzip2`: Enable `bzip2` decompression.
//!   - `zip-deflate`: Enable `flate2` decompression.
//! - `tar`: Read assets from TAR archives.
//!
//! ### Additional formats
//!
//! These features add support for asset formats. There is one feature per
//! format.
//!
//! - Serialisation formats (with [`serde`] crate): `bincode`, `json`,
//!   `msgpack`, `ron`, `toml`, `yaml`.
//! - Image formats (with [`image`] crate): `bmp`, `jpeg`, `png` `webp`.
//! - 3D formats (with [`gltf`] crate): `gltf`.
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
//! These features change inner data structures implementations.
//!
//! - [`parking_lot`]: Use `parking_lot`'s synchronization primitives.
//! - [`ahash`]: Use a faster hashing algorithm (enabled by default).
//!
//! # Basic example
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
//! modified or deleted, the values of all loaded assets that depend on this
//! file are automatically and transparently updated.
//!
//! To use hot-reloading, see [`AssetCache::hot_reload`].
//!
//! See the [`asset`] module for a precise description of how assets interact
//! with hot-reloading.
//!
//! # Ownership model
//!
//! You will notice that you cannot get owned [`Handle`]s, only references whose
//! lifetime are tied to that of the [`AssetCache`] from which there was loaded.
//! This may be seen as a weakness, as `'static` data is generally easier to
//! work with, but it is actually a clever use of Rust ownership system.
//!
//! As when you borrow an `&str` from a `String`, an `&Handle` guarantees
//! that the underlying asset is stored in the cache. This is especially useful
//! with hot-reloading: all `&Handle` are guaranteed to be reloaded when
//! possible, so two handles on the same asset always have the same value. This
//! would not be possible if `Handle`s were always `'static`.
//!
//! Note that this also means that you need a mutable reference to a cache to
//! remove assets from it.
//!
//! ## Getting owned data
//!
//! Working with owned data is far easier: you don't have to care about
//! lifetimes, it can easily be sent to other threads, etc. This section gives
//! a few techniques to work with the fact that caches give references to their
//! assets.
//!
//! Note that none of these proposals is compulsory to use this crate: you can
//! work with non-`'static` data, or invent your own techniques.
//!
//! ### Getting a `&'static AssetCache`
//!
//! Because the lifetime of a `Handle` reference is tied to that of the `&AssetCache`,
//! this makes possible to get `&'static Handle`s. Moreover, it enables you to
//! call [`AssetCache::enhance_hot_reloading`], which is easier to work with
//! than the default solution.
//!
//! You get easily get a `&'static AssetCache`, with the `once_cell` crate or
//! [`std::sync::OnceLock`], but you can also do it by [leaking a `Box`](Box::leak).
//!
//! Note that using this technique prevents you from removing assets from the
//! cache, so you have to keep them in memory for the duration of the program.
//! This also creates global state, which you might want to avoid.
//!
//! ### Cloning assets
//!
//! Assets being `'static` themselves, cloning them is a good way to opt out of
//! the lifetime of the cache. If cloning the asset itself is too expensive, you
//! can take advantage of the fact that `Arc<T>` is an asset if `T` is too and
//! that cloning an `Arc` is a rather cheap operation.
//!
//! Not that this usually does not work wery well with hot-reloading.
//!
//! ### Storing `String`s
//!
//! Strings are `'static` and easy to work with, and you can use them to load
//! an asset from the cache, which is a cheap operation if the asset is already
//! stored in it. If you want to ensure that no heavy operation is used, you
//! can do so with [`AssetCache::get_cached`].
//!
//! If you have to clone them a lot, you may consider changing your `String`
//! into an `Arc<str>` or a `SharedString` which is cheaper to clone.
//!
//! This is the technique internally used by `assets_manager` to store
//! directories.

#![warn(missing_docs, missing_debug_implementations)]
#![warn(unsafe_op_in_unsafe_fn)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate self as assets_manager;

pub mod asset;
pub use asset::{Asset, Compound, Storable};

mod cache;
pub use cache::AssetCache;

mod dirs;
pub use dirs::{Directory, RecursiveDirectory};

mod error;
pub use error::{BoxedError, Error};

pub mod loader;

mod map;

mod entry;
pub use entry::{AssetReadGuard, AtomicReloadId, Handle, ReloadId, ReloadWatcher, UntypedHandle};

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
/// - `"json"`: Use [`loader::JsonLoader`] and extension `.json`
/// - `"ron"`: Use [`loader::RonLoader`] and extension `.ron`
/// - `"toml"`: Use [`loader::TomlLoader`] and extension `.toml`
/// - `"txt"`: Use [`loader::ParseLoader`] and extension `.txt`
/// - `"yaml"` or `"yml"`: Use [`loader::YamlLoader`] and extensions `.yaml` and `.yml`
///
/// # Example
///
/// ```rust
/// # cfg_if::cfg_if! { if #[cfg(feature = "ron")] {
/// # use assets_manager::{Asset, AssetCache, BoxedError};
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
