//! Bytes sources to load assets from.
//!
//! This module contains the trait [`Source`](trait.Source.html), which allows
//! to specify how the files containing the assets are loaded. The struct
//! [`AssetCache`](../struct.AssetCache.html) is notably generic over a `Source`.
//!
//! This module also contains two built-in sources: [`FileSystem`](struct.FileSystem.html)
//! and [`Embedded`](struct.Embedded.html).

use std::{borrow::Cow, io};


mod filesystem;
pub use filesystem::FileSystem;


#[cfg(feature = "embedded")]
mod embedded;
#[cfg(feature = "embedded")]
pub use embedded::{Embedded, RawEmbedded};

/// Embed a directory in the binary
///
/// This macro takes as parameter the path of the directory to embed, and
/// returns a [`RawEmbedded`](struct.RawEmbedded.html), which can be used to
/// create an [`Embedded`](struct.Embedded.html) source.
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
/// # Usage
///
/// This trait's main usage is through an [`AssetCache`](../struct.AssetCache.html). You create a value which is `Source` and give it to `AssetCache`.
///
/// ## Example
///
/// Read assets from the filesystem or embed them in the binary depending on we
/// are building for WebAssembly:
///
/// ```no_run
/// use assets_manager::{AssetCache, source};
///
/// #[cfg(not(target_arch = "wasm32"))]
/// let source = source::FileSystem::new("assets")?;
///
/// #[cfg(target_arch = "wasm32")]
/// let source = source::Embedded::from(embed!("assets"));
///
/// let cache = AssetCache::with_source(source);
/// # Ok::<(), std::io::Error>(())
/// ```
pub trait Source {
    /// Try reading the source given an id and an extension.
    ///
    /// If no error occur, this function returns an `Cow`, which can be useful
    /// to avoid allocations.
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>>;

    /// Reads a directory given its id and an extension list.
    ///
    /// If no error occur, this function should return a list of file stems
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
    fn __private_hr_add_asset<A: crate::Asset>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_add_dir<A: crate::Asset>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_clear(&mut self) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_add_compound<A: crate::Compound>(&self, _: &str, _: crate::utils::DepsRecord) where Self: Sized {}
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

