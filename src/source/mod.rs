//! Bytes sources to load assets from.
//!
//! This module contains the trait [`Source`], which allows to specify how the
//! files containing the assets are loaded. The main usage usage of this trait
//! is with an [`AssetCache`].
//!
//! This module also contains two built-in sources: [`FileSystem`] and
//! [`Embedded`].
//!
//! # Hot-reloading
//!
//! Hot-reloading enable assets to be reloaded automatically when the source it
//! was loaded from was modified. It is only supported for the [`FileSystem`]
//! source at the moment.
//!
//! # Using a different source depending on the target platform
//!
//! There is no file system on WebAssembly, so you can for example choose to
//! embed your assets on this platform:
//!
//! ```no_run
//! use assets_manager::{AssetCache, source};
//!
//! #[cfg(not(target_arch = "wasm32"))]
//! let source = source::FileSystem::new("assets")?;
//!
//! #[cfg(target_arch = "wasm32")]
//! let source = source::Embedded::from(source::embed!("assets"));
//!
//! let cache = AssetCache::with_source(source);
//! # Ok::<(), std::io::Error>(())
//! ```

#[cfg(feature = "hot-reloading")]
use crate::utils::PrivateMarker;

use std::{borrow::Cow, io};

#[cfg(doc)]
use crate::AssetCache;

mod filesystem;
pub use filesystem::FileSystem;


#[cfg(feature = "embedded")]
mod embedded;
#[cfg(feature = "embedded")]
pub use embedded::{Embedded, RawEmbedded};

/// Embed a directory in the binary
///
/// This macro takes as parameter the path of the directory to embed, and
/// returns a [`RawEmbedded`], which can be used to create an [`Embedded`]
/// source.
///
/// ## Example
///
/// ```no_run
/// use assets_manager::{AssetCache, source::{embed, Embedded, RawEmbedded}};
///
/// static EMBEDDED: RawEmbedded<'static> = embed!("assets");
///
/// let embedded = Embedded::from(EMBEDDED);
/// let cache = AssetCache::with_source(embedded);
/// ```
#[cfg(feature = "embedded")]
#[cfg_attr(docsrs, doc(cfg(feature = "embedded")))]
pub use assets_manager_macros::embed;

#[cfg(test)]
mod tests;

/// Bytes sources to load assets from.
///
/// See [module-level documentation](super::source) for more informations.
pub trait Source {
    /// Try reading the source given an id and an extension.
    ///
    /// If no error occurs, this function returns an `Cow`, which can be useful
    /// to avoid allocations.
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>>;

    /// Reads a directory given its id and an extension list.
    ///
    /// If no error occurs, this function should return a list of file stems
    /// (without extension nor dir prefix) from files that have at least one of
    /// the given extensions.
    ///
    /// # Example
    ///
    /// ```
    /// use assets_manager::source::{FileSystem, Source};
    ///
    /// // In "assets/example" directory, there are "giant_bat.ron",
    /// // "goblin.ron", and other files that do not have "ron" extension.
    ///
    /// let fs = FileSystem::new("assets")?;
    /// let mut dir_content = fs.read_dir("example.monsters", &["ron"])?;
    ///
    /// // Order is important for equality comparison
    /// dir_content.sort();
    ///
    /// assert_eq!(dir_content, ["giant_bat", "goblin"]);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    fn read_dir(&self, id: &str, ext: &[&str]) -> io::Result<Vec<String>>;

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _add_asset<A: crate::Asset, P: PrivateMarker>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _add_dir<A: crate::Asset, P: PrivateMarker>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _clear<P: PrivateMarker>(&mut self) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _add_compound<A: crate::Compound, P: PrivateMarker>(&self, _: &str, _: crate::utils::DepsRecord) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _support_hot_reloading<P: PrivateMarker>() -> bool where Self: Sized {
        false
    }
}

impl<S> Source for Box<S>
where
    S: Source + ?Sized,
{
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>> {
        self.as_ref().read(id, ext)
    }

    fn read_dir(&self, dir: &str, ext: &[&str]) -> io::Result<Vec<String>> {
        self.as_ref().read_dir(dir, ext)
    }
}

