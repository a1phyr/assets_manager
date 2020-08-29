#[cfg(feature = "hot-reloading")]
use crate::{
    Asset,
    lock::Mutex,
    hot_reloading::{HotReloader, WatchedPaths}
};

use std::{
    borrow::Cow,
    fmt,
    fs,
    io,
    path::{Path, PathBuf},
};


#[inline]
pub fn extension_of(path: &Path) -> Option<&str> {
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
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<FileSystem> {
        let path = path.as_ref().canonicalize()?;
        let _ = path.read_dir()?;

        #[cfg(feature = "hot-reloading")]
        let reloader = Mutex::new(HotReloader::start(&path).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?);

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

impl super::Source for FileSystem {
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
