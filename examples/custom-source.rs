//! This example shows an implementation of hot-reloading for a custom
//! [`Source`].
//!
//! Note that it uses the built-in [`FsWatcherBuilder`], which makes it easier
//! but only works with the file system. For more advanced uses, please look at
//! the code of FsWatcherBuilder`.

use assets_manager::{
    hot_reloading::{DynUpdateSender, EventSender, FsWatcherBuilder},
    source::{DirEntry, FileContent, FileSystem, Source},
    AssetCache, BoxedError,
};
use std::{
    io,
    path::{Path, PathBuf},
};

/// Loads assets from the default path or `ASSETS_OVERRIDE` env if it is set.
#[derive(Debug, Clone)]
pub struct FsWithOverride {
    default_dir: FileSystem,
    override_dir: Option<FileSystem>,
}

impl FsWithOverride {
    pub fn new<P: AsRef<Path>>(default_path: P) -> io::Result<Self> {
        // Try override path
        let default_dir = FileSystem::new(default_path)?;
        let override_dir = std::env::var_os("ASSETS_OVERRIDE").and_then(|path| {
            FileSystem::new(path)
                .map_err(|err| log::error!("Error setting override assets directory: {err}"))
                .ok()
        });

        Ok(Self {
            default_dir,
            override_dir,
        })
    }

    pub fn path_of(&self, specifier: &str, ext: &str) -> PathBuf {
        self.default_dir.path_of(DirEntry::File(specifier, ext))
    }
}

impl Source for FsWithOverride {
    fn read(&self, id: &str, ext: &str) -> io::Result<FileContent> {
        // Try override path
        if let Some(dir) = &self.override_dir {
            match dir.read(id, ext) {
                Ok(content) => return Ok(content),
                Err(err) => {
                    if err.kind() != io::ErrorKind::NotFound {
                        let path = dir.path_of(DirEntry::File(id, ext));
                        log::warn!("Error reading \"{}\": {err}", path.display());
                    }
                }
            }
        }

        // If not found in override path, try load from main asset path
        self.default_dir.read(id, ext)
    }

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        if let Some(dir) = &self.override_dir {
            match dir.read_dir(id, f) {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if err.kind() != io::ErrorKind::NotFound {
                        let path = dir.path_of(DirEntry::Directory(id));
                        log::warn!("Error reading \"{}\": {}", path.display(), err);
                    }
                }
            }
        }

        // Try from main asset path
        self.default_dir.read_dir(id, f)
    }

    fn exists(&self, entry: DirEntry) -> bool {
        self.override_dir
            .as_ref()
            .map_or(false, |dir| dir.exists(entry))
            || self.default_dir.exists(entry)
    }

    // Here is the hot-reloading magic
    fn configure_hot_reloading(&self, events: EventSender) -> Result<DynUpdateSender, BoxedError> {
        let mut builder = FsWatcherBuilder::new()?;

        // Register watched directories
        if let Some(dir) = &self.override_dir {
            builder.watch(dir.root().to_owned())?;
        }
        builder.watch(self.default_dir.root().to_owned())?;

        // Start hot-reloading with our paths
        Ok(builder.build(events))
    }

    fn make_source(&self) -> Option<Box<dyn Source + Send>> {
        Some(Box::new(self.clone()))
    }
}

fn main() -> Result<(), BoxedError> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let source = FsWithOverride::new("assets")?;
    let cache = AssetCache::with_source(source);

    let msg = cache.load::<String>("example.hello")?;

    loop {
        #[cfg(feature = "hot-reloading")]
        cache.hot_reload();

        println!("{}", msg.read());
        std::thread::sleep(std::time::Duration::from_secs(1))
    }
}
