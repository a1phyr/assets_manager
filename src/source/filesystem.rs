use crate::{hot_reloading::HotReloader, utils::extension_of};

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
#[derive(Clone)]
pub struct FileSystem {
    path: PathBuf,
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
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<FileSystem> {
        let path = path.as_ref().canonicalize()?;
        let _ = path.read_dir()?;

        Ok(FileSystem { path })
    }

    /// Gets the path of the source's root.
    ///
    /// The path is currently given as absolute, but this may change in the future.
    #[inline]
    pub fn root(&self) -> &Path {
        &self.path
    }

    /// Returns the path that the directory entry would have if it exists.
    #[inline]
    pub fn path_of(&self, entry: DirEntry) -> PathBuf {
        crate::utils::path_of_entry(&self.path, entry)
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

    fn configure_hot_reloading(&self) -> Result<Option<HotReloader>, crate::BoxedError> {
        #[cfg(feature = "hot-reloading")]
        {
            let mut watcher = crate::hot_reloading::FsWatcherBuilder::new()?;
            watcher.watch(self.path.clone())?;
            let config = watcher.build();
            Ok(Some(HotReloader::start(config, self.clone())))
        }

        #[cfg(not(feature = "hot-reloading"))]
        {
            Ok(None)
        }
    }
}

impl fmt::Debug for FileSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSystem")
            .field("root", &self.path)
            .finish()
    }
}
