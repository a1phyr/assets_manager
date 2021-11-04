#[cfg(feature = "hot-reloading")]
use crate::hot_reloading::HotReloader;
use crate::utils::extension_of;

#[cfg(doc)]
use crate::AssetCache;

use std::{
    borrow::Cow,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use super::{DirEntry, Source};

/// A [`Source`] to load assets from a directory in the file system.
///
/// This is the default `Source` of [`AssetCache`].
///
/// ## Hot-reloading
///
/// This source supports hot-reloading: when a file is edited, the corresponding
/// assets are reloaded when [`AssetCache::hot_reload`] is called.
///
/// ## WebAssembly
///
/// This source does not work in WebAssembly, because there is no file system.
/// When called, it always returns an error.
pub struct FileSystem {
    path: PathBuf,

    #[cfg(feature = "hot-reloading")]
    pub(crate) reloader: Option<HotReloader>,
}

impl FileSystem {
    /// Creates a new `FileSystem` from a directory.
    ///
    /// Generally you do not need to call this function directly, as the
    /// [`AssetCache::new`] method provides a shortcut to create a cache
    /// reading from the filesystem.
    ///
    /// # Errors
    ///
    /// An error can occur if `path` is not a valid readable directory.
    ///
    /// If hot-reloading fails to start (if feature `hot-reloading` is used),
    /// an error is logged and this function returns `Ok`.
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<FileSystem> {
        Self::_new(path.as_ref(), true)
    }

    /// Same as `new`, but does not start hot-reloading.
    ///
    /// If feature `hot-reloading` is not enabled, this function is equivalent
    /// to `new`.
    pub fn without_hot_reloading<P: AsRef<Path>>(path: P) -> io::Result<FileSystem> {
        Self::_new(path.as_ref(), false)
    }

    fn _new(path: &Path, _hot_reloading: bool) -> io::Result<FileSystem> {
        let path = path.canonicalize()?;
        let _ = path.read_dir()?;

        #[cfg(feature = "hot-reloading")]
        let reloader = if _hot_reloading {
            match HotReloader::start(&path) {
                Ok(r) => Some(r),
                Err(err) => {
                    log::error!("Unable to start hot-reloading: {}", err);
                    None
                }
            }
        } else {
            None
        };

        Ok(FileSystem {
            path,

            #[cfg(feature = "hot-reloading")]
            reloader,
        })
    }

    /// Gets the path of the source's root.
    ///
    /// The path is currently given as absolute, but this may change in the future.
    #[inline]
    pub fn root(&self) -> &Path {
        &self.path
    }

    /// Returns the path that the directory entry would have if it exists.
    pub fn path_of(&self, entry: DirEntry) -> PathBuf {
        let mut path = self.path.clone();
        path.extend(entry.id().split('.'));
        if let DirEntry::File(_, ext) = entry {
            path.set_extension(ext);
        }
        path
    }
}

impl Source for FileSystem {
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>> {
        let path = self.path_of(DirEntry::File(id, ext));
        fs::read(path).map(Into::into)
    }

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        let dir_path = self.path_of(DirEntry::Directory(id));
        let entries = fs::read_dir(dir_path)?;

        let mut entry_id = id.to_owned();

        // Ignore entries that return an error
        for entry in entries.flatten() {
            let path = entry.path();

            let name = match path.file_stem().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => continue,
            };

            let this_id: &str = if !id.is_empty() {
                entry_id.truncate(id.len());
                entry_id.extend([".", name].iter().copied());
                &entry_id
            } else {
                name
            };

            if path.is_file() {
                if let Some(ext) = extension_of(&path) {
                    f(DirEntry::File(this_id, ext));
                }
            } else if path.is_dir() {
                f(DirEntry::Directory(this_id));
            }
        }

        Ok(())
    }

    fn exists(&self, entry: DirEntry) -> bool {
        self.path_of(entry).exists()
    }

    #[cfg(feature = "hot-reloading")]
    fn _private_path_of(&self, entry: DirEntry) -> PathBuf {
        self.path_of(entry)
    }

    #[cfg(feature = "hot-reloading")]
    fn _private_send_message(&self, msg: crate::hot_reloading::PublicUpdateMessage) {
        if let Some(reloader) = &self.reloader {
            reloader.send_update(msg.0);
        }
    }

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn _private_supports_hot_reloading(&self) -> bool {
        self.reloader.is_some()
    }
}

impl fmt::Debug for FileSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSystem")
            .field("root", &self.path)
            .finish()
    }
}
