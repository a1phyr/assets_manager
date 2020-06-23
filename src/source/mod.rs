//! TODO


#[cfg(test)]
mod tests;


#[cfg(feature = "hot-reloading")]
use crate::{
    Asset,
    lock::Mutex,
    hot_reloading::{HotReloader, WatchedPaths}
};

use std::{
    borrow::Cow,
    error::Error,
    fmt,
    fs,
    io,
    path::{Path, PathBuf},
};


/// TODO
pub trait Source {
    /// TODO
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>>;

    /// TODO
    fn read_dir(&self, dir: &str, ext: &[&str]) -> io::Result<Vec<String>>;

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_add_asset<A: Asset>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_add_dir<A: Asset>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_clear(&mut self) where Self: Sized {}
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

#[inline]
pub(crate) fn extension_of(path: &Path) -> Option<&str> {
    match path.extension() {
        Some(ext) => ext.to_str(),
        None => Some(""),
    }
}

#[inline]
fn has_extension(path: &Path, ext: &[&str]) -> bool {
    match extension_of(path) {
        Some(file_ext) => ext.contains(&file_ext),
        None => false,
    }
}

/// TODO
pub struct FileSystem {
    path: PathBuf,

    #[cfg(feature = "hot-reloading")]
    pub(crate) reloader: Mutex<HotReloader>,
    #[cfg(feature = "hot-reloading")]
    pub(crate) watched: Mutex<WatchedPaths>,
}

impl FileSystem {
    /// TODO
    pub fn new<P: AsRef<Path>>(path: P) -> Result<FileSystem, CacheError> {
        let path = path.as_ref().canonicalize().map_err(ErrorKind::Io)?;
        let _ = path.read_dir().map_err(ErrorKind::Io)?;

        #[cfg(feature = "hot-reloading")]
        let reloader = Mutex::new(HotReloader::start(&path).map_err(ErrorKind::Notify)?);

        Ok(FileSystem {
            path,

            #[cfg(feature = "hot-reloading")]
            reloader,
            #[cfg(feature = "hot-reloading")]
            watched: Mutex::new(WatchedPaths::new()),
        })
    }

    /// Gets the path of the source's root.
    ///
    /// The path is currently given as absolute, but this may change in the future.
    #[inline]
    pub fn root(&self) -> &Path {
        &self.path
    }

    /// TODO
    pub fn path_of(&self, id: &str, ext: &str) -> PathBuf {
        let mut path = self.path.clone();
        path.extend(id.split('.'));
        path.set_extension(ext);
        path
    }
}

impl Source for FileSystem {
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>> {
        let path = self.path_of(id, ext);
        fs::read(path).map(Into::into)
    }

    fn read_dir(&self, dir: &str, ext: &[&str]) -> io::Result<Vec<String>> {
        let dir_path = self.path_of(dir, "");
        let entries = fs::read_dir(dir_path)?;

        let mut loaded = Vec::new();

        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();

                if !has_extension(&path, ext) {
                    continue;
                }

                let name = match path.file_stem().and_then(|n| n.to_str()) {
                    Some(name) => name,
                    None => continue,
                };

                if path.is_file() {
                    loaded.push(name.into());
                }
            }
        }

        Ok(loaded)
    }

    #[cfg(feature = "hot-reloading")]
    fn __private_hr_add_asset<A: Asset>(&self, id: &str) {
        for ext in A::EXTENSIONS {
            let path = self.path_of(id, ext);
            self.watched.lock().add_file::<A>(path, id.into());
        }
    }

    #[cfg(feature = "hot-reloading")]
    fn __private_hr_add_dir<A: Asset>(&self, id: &str) {
        let path = self.path_of(id, "");
        self.watched.lock().add_dir::<A>(path, id.into());
    }

    #[cfg(feature = "hot-reloading")]
    fn __private_hr_clear(&mut self) {
        self.watched.get_mut().clear();
    }
}

impl fmt::Debug for FileSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSystem").field("root", &self.path).finish()
    }
}


enum ErrorKind {
    Io(io::Error),
    #[cfg(feature = "hot-reloading")]
    Notify(notify::Error),
}

/// An error which occurs when creating a cache.
///
/// This error can be returned by [`AssetCache::new`].
///
/// [`AssetCache::new`]: struct.AssetCache.html#method.new
pub struct CacheError(ErrorKind);

impl From<ErrorKind> for CacheError {
    fn from(err: ErrorKind) -> Self {
        Self(err)
    }
}

impl fmt::Debug for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_tuple("CacheError");

        match &self.0 {
            ErrorKind::Io(err) => debug.field(err),
            #[cfg(feature = "hot-reloading")]
            ErrorKind::Notify(err) => debug.field(err),
        }.finish()
    }
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::Io(err) => err.fmt(f),
            #[cfg(feature = "hot-reloading")]
            ErrorKind::Notify(err) => err.fmt(f),
        }
    }
}

impl Error for CacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.0 {
            ErrorKind::Io(err) => Some(err),
            #[cfg(feature = "hot-reloading")]
            ErrorKind::Notify(err) => Some(err),
        }
    }
}
