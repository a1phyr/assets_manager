//! Bytes sources to load assets from.
//!
//! This module contains the trait [`Source`], which allows to specify how the
//! files containing the assets are loaded. The main usage usage of this trait
//! is with an [`AssetCache`].
//!
//! This module also contains three built-in sources: [`FileSystem`], [`Zip`]
//! and [`Embedded`].
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
use crate::{asset::DirLoadable, AssetCache};

mod filesystem;
pub use filesystem::FileSystem;


#[cfg(feature = "embedded")]
mod embedded;
#[cfg(feature = "embedded")]
pub use embedded::{Embedded, RawEmbedded};


#[cfg(feature = "zip")]
mod _zip;
#[cfg(feature = "zip")]
pub use _zip::Zip;

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

/// An entry in a source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirEntry<'a> {
    /// A file with an id and an extension.
    File(&'a str, &'a str),

    /// A directory with an id.
    Directory(&'a str),
}

impl<'a> DirEntry<'a> {
    /// Returns `true` if this is a `File`.
    #[inline]
    pub const fn is_file(&self) -> bool {
        matches!(self, DirEntry::File(..))
    }

    /// Returns `true` if this is a `Directory`.
    #[inline]
    pub const fn is_dir(&self) -> bool {
        matches!(self, DirEntry::Directory(_))
    }

    /// Returns the id of the pointed entity.
    #[inline]
    pub const fn id(self) -> &'a str {
        match self {
            DirEntry::File(id, _) => id,
            DirEntry::Directory(id) => id,
        }
    }

    /// Returns the entry's parent's id, or `None` if it is the root.
    ///
    /// # Example
    ///
    /// ```
    /// use assets_manager::source::DirEntry;
    ///
    /// let entry = DirEntry::File("example.hello.world", "txt");
    /// assert_eq!(entry.parent_id(), Some("example.hello"));
    ///
    /// let root = DirEntry::Directory("");
    /// assert!(root.parent_id().is_none());
    /// ```
    #[inline]
    pub fn parent_id(self) -> Option<&'a str> {
        let id = self.id();
        if id.is_empty() {
            None
        } else {
            match id.rfind('.') {
                Some(n) => Some(&id[..n]),
                None => Some(""),
            }
        }
    }
}

/// Bytes sources to load assets from.
///
/// This trait provides an abstraction over a basic filesystem, which is used to
/// load assets independantly from the actual storage kind.
///
/// As a consumer of this library, you generally don't need to use this trait,
/// exept when implementing [`DirLoadable`].
///
/// See [module-level documentation](super::source) for more informations.
pub trait Source {
    /// Try reading the source given an id and an extension.
    ///
    /// If no error occurs, this function returns an `Cow`, which can be useful
    /// to avoid allocations.
    ///
    /// Most of the time, you won't need to use this method, directly, as it is
    /// done for you by an [`AssetCache`] when you load [`Asset`]s.
    ///
    /// [`Asset`]: crate::Asset
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>>;

    /// Reads the content of a directory.
    ///
    /// If no error occurs, this function executes the given closure for each
    /// entry in the directory.
    ///
    /// # Example
    ///
    /// ```
    /// use assets_manager::source::{DirEntry, FileSystem, Source};
    ///
    /// // In "assets/example" directory, there are "giant_bat.ron",
    /// // "goblin.ron", and other files that do not have "ron" extension.
    ///
    /// let fs = FileSystem::new("assets")?;
    ///
    /// let mut dir_content = Vec::new();
    /// fs.read_dir("example.monsters", &mut |entry| {
    ///     if let DirEntry::File(id, ext) = entry {
    ///         if ext == "ron" {
    ///             dir_content.push(id.to_owned());
    ///         }
    ///     }
    /// })?;
    ///
    /// // Sort for equality comparison
    /// dir_content.sort();
    ///
    /// assert_eq!(dir_content, ["example.monsters.giant_bat", "example.monsters.goblin"]);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()>;

    /// Returns `true` if the entry points at an existing entity.
    ///
    /// # Example
    ///
    /// ```
    /// use assets_manager::source::{DirEntry, FileSystem, Source};
    ///
    /// let fs = FileSystem::new("assets")?;
    ///
    /// assert!(fs.exists(DirEntry::File("example.monsters.goblin", "ron")));
    /// assert!(!fs.exists(DirEntry::File("example.monsters.spider", "ron")));
    /// # Ok::<(), std::io::Error>(())
    /// ```
    fn exists(&self, entry: DirEntry) -> bool;

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _add_asset<A: crate::Asset, P: PrivateMarker>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _clear<P: PrivateMarker>(&mut self) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _add_compound<A: crate::Compound, P: PrivateMarker>(&self, _: &str, _: crate::utils::DepsRecord) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _support_hot_reloading<P: PrivateMarker>(&self) -> bool where Self: Sized {
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

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        self.as_ref().read_dir(id, f)
    }

    fn exists(&self, entry: DirEntry) -> bool {
        self.as_ref().exists(entry)
    }
}

