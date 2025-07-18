//! Bytes sources to load assets from.
//!
//! This module contains the [`Source`] trait, which defines how files are read.
//! The primary use of this trait is with [`AssetCache`] to provide flexible
//! asset loading from different storage backends.
//!
//! Built-in sources include:
//! - [`FileSystem`]: Load from local filesystem
//! - [`Zip`]: Load from ZIP archives
//! - [`Tar`]: Load from TAR archives
//! - [`Embedded`]: Load from assets embedded in the binary
//!
//! # Hot-reloading
//!
//! Hot-reloading enables assets to be automatically reloaded when their source files
//! are modified. This functionality requires the `Source` implementation to support it.
//! The built-in [`FileSystem`] source provides hot-reloading support out of the box.
//!
//! # Platform-specific source selection
//!
//! Since WebAssembly doesn't have filesystem access, you can conditionally use different
//! sources based on the target platform. For example, use filesystem on native platforms
//! and embedded assets on WASM:
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

use std::{borrow::Cow, fmt, io};

#[cfg(doc)]
use crate::{Asset, AssetCache, asset::DirLoadable};
use crate::{BoxedError, SharedString, hot_reloading::EventSender};

mod filesystem;
pub use filesystem::FileSystem;

#[cfg(feature = "embedded")]
mod embedded;
#[cfg(feature = "embedded")]
#[cfg_attr(docsrs, doc(cfg(feature = "embedded")))]
pub use embedded::{Embedded, RawEmbedded};

#[cfg(feature = "tar")]
mod tar;
#[cfg(feature = "tar")]
#[cfg_attr(docsrs, doc(cfg(feature = "tar")))]
pub use self::tar::Tar;

#[cfg(feature = "zip")]
mod zip;
#[cfg(feature = "zip")]
#[cfg_attr(docsrs, doc(cfg(feature = "zip")))]
pub use self::zip::Zip;

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

/// An owned version of a `DirEntry`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OwnedDirEntry {
    /// A file with an id and an extension.
    File(SharedString, SharedString),

    /// A directory with an id.
    Directory(SharedString),
}

impl OwnedDirEntry {
    /// Converts to a borrowed `DirEntry`.
    pub fn as_dir_entry(&self) -> DirEntry<'_> {
        match self {
            OwnedDirEntry::File(id, ext) => DirEntry::File(id, ext),
            OwnedDirEntry::Directory(id) => DirEntry::Directory(id),
        }
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn into_dependency(self) -> crate::hot_reloading::Dependency {
        match self {
            OwnedDirEntry::File(id, ext) => crate::hot_reloading::Dependency::File(id, ext),
            OwnedDirEntry::Directory(id) => crate::hot_reloading::Dependency::Directory(id),
        }
    }
}

/// A handle to an immutable memory mapped buffer.
#[cfg(feature = "mmap")]
#[cfg_attr(docsrs, doc(cfg(feature = "mmap")))]
#[derive(Debug)]
pub struct Mmap(memmap2::Mmap);

#[cfg(feature = "mmap")]
impl Mmap {
    /// Creates a read-only memory map backed by a file.
    ///
    /// ## Safety
    ///
    /// All file-backed memory map constructors are marked `unsafe` because of the potential for
    /// *Undefined Behavior* (UB) using the map if the underlying file is subsequently modified, in or
    /// out of process. Applications must consider the risk and take appropriate precautions when using
    /// file-backed maps. Solutions such as file permissions, locks or process-private (e.g. unlinked)
    /// files exist but are platform specific and limited.
    pub unsafe fn map(file: &std::fs::File) -> io::Result<Self> {
        let map = unsafe { memmap2::Mmap::map(file)? };
        Ok(Self(map))
    }
}

#[cfg(feature = "mmap")]
impl AsRef<[u8]> for Mmap {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// The raw content of a file.
///
/// This enum enables returning the raw bytes of a file in the most efficient
/// representation.
pub enum FileContent<'a> {
    /// The content of the file as a borrowed byte slice.
    Slice(&'a [u8]),

    /// The content of the file as an owned buffer.
    Buffer(Vec<u8>),

    /// The content of the file as an owned value that contains bytes.
    Owned(Box<dyn AsRef<[u8]> + 'a>),
}

impl<'a> FileContent<'a> {
    /// Creates a `FileContent` from an owned value that contains bytes.
    #[inline(always)]
    pub fn from_owned(x: impl AsRef<[u8]> + 'a) -> Self {
        Self::Owned(Box::new(x))
    }

    #[inline]
    pub(crate) fn with_cow<T>(self, f: impl FnOnce(Cow<[u8]>) -> T) -> T {
        match self {
            FileContent::Slice(b) => f(Cow::Borrowed(b)),
            FileContent::Buffer(b) => f(Cow::Owned(b)),
            FileContent::Owned(b) => f(Cow::Borrowed((*b).as_ref())),
        }
    }
}

impl fmt::Debug for FileContent<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileContent { .. }")
    }
}

impl AsRef<[u8]> for FileContent<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Slice(b) => b,
            Self::Buffer(b) => b,
            Self::Owned(b) => (**b).as_ref(),
        }
    }
}

impl<'a> From<&'a [u8]> for FileContent<'a> {
    #[inline]
    fn from(slice: &'a [u8]) -> Self {
        Self::Slice(slice)
    }
}

impl From<Vec<u8>> for FileContent<'_> {
    #[inline]
    fn from(buffer: Vec<u8>) -> Self {
        Self::Buffer(buffer)
    }
}

mod private {
    #[derive(Debug)]
    pub struct Private;
}

/// Bytes sources to load assets from.
///
/// This trait provides an abstraction over filesystem operations, allowing assets to be
/// loaded independently of their storage backend (filesystem, archive, embedded, etc.).
///
/// As a consumer of this library, you typically only need to use this trait
/// directly when implementing [`Asset`] or [`DirLoadable`].
///
/// See [module-level documentation](super::source) for more information.
pub trait Source {
    /// Try reading the source given an id and an extension.
    ///
    /// If no error occurs, this function returns the raw content of the file as
    /// a  [`FileContent`], so it can avoid copying bytes around if possible.
    ///
    /// Most of the time, you won't need to use this method, directly, as it is
    /// done for you by an [`AssetCache`] when you load [`Asset`]s.
    ///
    /// [`Asset`]: crate::Asset
    fn read(&self, id: &str, ext: &str) -> io::Result<FileContent<'_>>;

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

    /// Starts hot-reloading.
    ///
    /// This method receives an `EventSender` to notify the hot-reloading
    /// subsystem when assets should be reloaded.
    ///
    /// The returned result is there purely for conveniency: if this function
    /// returns an error, it is logged and nothing more is done with it.
    ///
    /// The default implementation does nothing and returns `Ok(())`.
    #[inline]
    fn configure_hot_reloading(&self, _events: EventSender) -> Result<(), BoxedError> {
        Ok(())
    }

    #[doc(hidden)]
    #[inline]
    fn type_id(&self, _: private::Private) -> std::any::TypeId
    where
        Self: 'static,
    {
        std::any::TypeId::of::<Self>()
    }
}

impl dyn Source + Send + Sync {
    #[inline]
    pub(crate) fn downcast_ref<S: Source + 'static>(&self) -> Option<&S> {
        if self.type_id(private::Private) == std::any::TypeId::of::<S>() {
            unsafe { Some(&*(self as *const dyn Source as *const S)) }
        } else {
            None
        }
    }
}

impl<S> Source for Box<S>
where
    S: Source + ?Sized,
{
    #[inline]
    fn read(&self, id: &str, ext: &str) -> io::Result<FileContent<'_>> {
        self.as_ref().read(id, ext)
    }

    #[inline]
    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        self.as_ref().read_dir(id, f)
    }

    #[inline]
    fn exists(&self, entry: DirEntry) -> bool {
        self.as_ref().exists(entry)
    }

    #[inline]
    fn configure_hot_reloading(&self, events: EventSender) -> Result<(), BoxedError> {
        self.as_ref().configure_hot_reloading(events)
    }
}

impl<S> Source for &S
where
    S: Source + ?Sized,
{
    #[inline]
    fn read(&self, id: &str, ext: &str) -> io::Result<FileContent<'_>> {
        (**self).read(id, ext)
    }

    #[inline]
    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        (**self).read_dir(id, f)
    }

    #[inline]
    fn exists(&self, entry: DirEntry) -> bool {
        (**self).exists(entry)
    }

    #[inline]
    fn configure_hot_reloading(&self, events: EventSender) -> Result<(), BoxedError> {
        (**self).configure_hot_reloading(events)
    }
}

impl<S> Source for std::sync::Arc<S>
where
    S: Source + ?Sized,
{
    #[inline]
    fn read(&self, id: &str, ext: &str) -> io::Result<FileContent<'_>> {
        self.as_ref().read(id, ext)
    }

    #[inline]
    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        self.as_ref().read_dir(id, f)
    }

    #[inline]
    fn exists(&self, entry: DirEntry) -> bool {
        self.as_ref().exists(entry)
    }

    #[inline]
    fn configure_hot_reloading(&self, events: EventSender) -> Result<(), BoxedError> {
        self.as_ref().configure_hot_reloading(events)
    }
}

/// A [`Source`] that contains nothing.
///
/// Calling `read` or `read_dir` from this source will always return an error.
#[derive(Debug)]
pub struct Empty;

impl Source for Empty {
    #[inline]
    fn read(&self, _id: &str, _ext: &str) -> io::Result<FileContent<'_>> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    #[inline]
    fn read_dir(&self, _id: &str, _f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    #[inline]
    fn exists(&self, _entry: DirEntry) -> bool {
        false
    }
}

impl Default for Empty {
    #[inline]
    fn default() -> Self {
        Self
    }
}
