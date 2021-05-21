//! Conveniently load, store and cache external resources.
//!
//! This crate aims at providing a filesystem abstraction to easily load external resources.
//! It was originally thought for games, but can, of course, be used in other contexts.
//!
//! The structure [`AssetCache`] is the entry point of the crate.
//!
//! ## Cargo features
//!
//! - `hot-reloading`: Add hot-reloading
//!
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
//!
//! ### Additional formats
//!
//! These features add support for asset formats. There is one feature per
//! format.
//!
//! - Serialisation formats (with [`serde`] crate): `bincode`, `cbor`, `json`,
//! `msgpack`, `ron`, `toml`, `yaml`.
//! - Audio formats (with [`rodio`] crate): `mp3`, `flac`, `vorbis`, `wav`.
//! - Image formats (with [`image`] crate): `bmp`, `jpeg`, `png`.
//!
//! ### Internal features
//!
//! These features change inner data structures implementations. They usually
//! increase performances, and are therefore enabled by default.
//!
//! - [`parking_lot`]: Use improved synchronization primitives.
//! - [`ahash`]: Use a faster hashing algorithm.
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
//! assert!(other_handle.ptr_eq(&handle));
//! # }}
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Borrowship model
//!
//! You will notice that an [`Handle`] is not `'static`: its lifetime is
//! tied to that of the [`AssetCache`] from which it was loaded. This may be
//! seen as a weakness, as `'static` data is generally easier to work with, but
//! it is actually a clever use of Rust ownership system.
//!
//! As when you borrow an `&str` from a `String`, an `Handle` guarantees
//! that the underlying asset is stored in the cache. This is especially useful
//! with hot-reloading: all `Handle` are guaranteed to be reloaded when
//! possible, so two handles on the same asset always have the same value. This
//! would not be possible if `Handle`s were always `'static`.
//!
//! Note that this also means that you need a mutable reference on a cache to
//! remove assets from it.
//!
//! ## Becoming `'static`
//!
//! Working with `'static` data is far easier: you don't have to care about
//! lifetimes, they can easily be sent in other threads, etc. So, how to get
//! `'static` data from `Handle`s ?
//!
//! Note that none of these proposals is compulsory to use this crate: you can
//! work with non-`'static` data, or invent your own techniques.
//!
//! ### Getting a `&'static AssetCache`
//!
//! The lifetime of an `Handle` being tied to that of the `&AssetCache`,
//! this enables you to get `'static` `Handle`s. Moreover, it enables you
//! to call [`AssetCache::enhance_hot_reloading`], which is easier to work with
//! and has better performances than the default solution.
//!
//! You get easily get a `&'static AssetCache`, with the `lazy_static` crate,
//! but you can also do it by [leaking a `Box`].
//!
//! Note that using this technique prevents you from removing assets from the
//! cache, so you have to keep them in memory for the duration of the program.
//! This also creates global state, which you might want to avoid.
//!
//! [leaking a `Box`]: https://doc.rust-lang.org/std/boxed/struct.Box.html#method.leak
//!
//! ### Cloning assets
//!
//! Assets being `'static` themselves, cloning them is a good way to opt out of
//! the lifetime of the cache. If cloning the asset itself is too expensive,
//! you can take advantage of the fact that `Arc<A>` is an asset if `A` is too:
//! cloning an `Arc` is a rather cheap operation (at least, when compared to
//! allocating memory).
//!
//! However, by doing so, you explicitly opt-out hot-reloading, which is done
//! via `Handle`s. This can also be a benefit if you need to ensure that your
//! data does not change spuriously.
//!
//! ### Storing `String`s
//!
//! Strings are `'static` and easy to work with, and you can use them to load
//! an asset from the cache, which is a cheap operation if the asset is already
//! stored in it. If you want to ensure that no heavy operation is used, you
//! can do so with [`AssetCache::load_cached`].
//!
//! If you have to clone them a lot, you may consider changing your `String`
//! into an `Arc<str>` which is cheaper to clone.
//!
//! This is the technique internally used by `assets_manager` to store cached
//! directories.

#![doc(html_root_url = "https://docs.rs/assets_manager/0.4.4")]

#![warn(
    missing_docs,
    missing_debug_implementations,
)]

#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate self as assets_manager;

pub mod asset;
pub use asset::{Asset, Compound};

mod cache;
pub use cache::AssetCache;

mod dirs;
pub use dirs::DirHandle;

mod error;
pub use error::{BoxedError, Error};

pub mod loader;

mod entry;
pub use entry::{AssetGuard, Handle};

pub mod source;

#[cfg(feature = "hot-reloading")]
mod hot_reloading;

pub mod utils;

#[cfg(test)]
mod tests;
