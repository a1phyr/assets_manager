//! Definition of the cache

use crate::{
    Asset, Error, Compound, Handle,
    dirs::{CachedDir, DirReader},
    entry::CacheEntry,
    loader::Loader,
    utils::{HashMap, Key, OwnedKey, Private, RwLock},
    source::{FileSystem, Source},
};

#[cfg(doc)]
use crate::{AssetGuard, ReadDir, ReadAllDir};

use std::{
    fmt,
    io,
    path::Path,
};

#[cfg(feature = "hot-reloading")]
use crate::utils::HashSet;

#[cfg(feature = "hot-reloading")]
use std::{
    cell::Cell,
    ptr::NonNull,
};


#[cfg(feature = "hot-reloading")]
struct Record {
    cache: usize,
    records: HashSet<OwnedKey>,
}

#[cfg(feature = "hot-reloading")]
thread_local! {
    static RECORDING: Cell<Option<NonNull<Record>>> = Cell::new(None);
}

/// The main structure of this crate, used to cache assets.
///
/// It uses interior mutability, so assets can be added in the cache without
/// requiring a mutable reference, but one is required to remove an asset.
///
/// Within the cache, assets are identified with their type and a string. This
/// string is constructed from the asset path, replacing `/` by `.` and removing
/// the extension. Given that, you cannot use `.` in your file names except for
/// the extension.
///
/// **Note**: Using symbolic or hard links within the cached directory can lead
/// to surprising behavior (especially with hot-reloading), and thus should be
/// avoided.
///
/// # Example
///
/// ```
/// # cfg_if::cfg_if! { if #[cfg(feature = "ron")] {
/// use assets_manager::{Asset, AssetCache, loader};
/// use serde::Deserialize;
///
/// #[derive(Debug, Deserialize)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
///
/// impl Asset for Point {
///     const EXTENSION: &'static str = "ron";
///     type Loader = loader::RonLoader;
/// }
///
/// // Create a cache
/// let cache = AssetCache::new("assets")?;
///
/// // Get an asset from the file `assets/common/position.ron`
/// let point_handle = cache.load::<Point>("common.position")?;
///
/// // Read it
/// let point = point_handle.read();
/// println!("Loaded position: {:?}", point);
/// # assert_eq!(point.x, 5);
/// # assert_eq!(point.y, -6);
///
/// // Release the lock
/// drop(point);
///
/// // Use hot-reloading
/// loop {
///     println!("Position: {:?}", point_handle.read());
/// #   #[cfg(feature = "hot-reloading")]
///     cache.hot_reload();
/// #   break;
/// }
///
/// # }}
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct AssetCache<S=FileSystem> {
    source: S,

    pub(crate) assets: RwLock<HashMap<OwnedKey, CacheEntry>>,
    pub(crate) dirs: RwLock<HashMap<OwnedKey, CachedDir>>,
}

impl AssetCache<FileSystem> {
    /// Creates a cache that loads assets from the given directory.
    ///
    /// # Errors
    ///
    /// An error will be returned if `path` is not valid readable directory or
    /// if hot-reloading failed to start (if feature `hot-reloading` is used).
    #[inline]
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<AssetCache<FileSystem>> {
        let source = FileSystem::new(path)?;
        Ok(Self::with_source(source))
    }
}

impl<S> AssetCache<S>
where
    S: Source,
{
    /// Creates a cache that loads assets from the given source.
    pub fn with_source(source: S) -> AssetCache<S> {
        AssetCache {
            assets: RwLock::new(HashMap::new()),
            dirs: RwLock::new(HashMap::new()),

            source,
        }
    }

    /// Returns a reference to the cache's [`Source`].
    #[inline]
    pub fn source(&self) -> &S {
        &self.source
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn record_load<A: Compound>(&self, id: &str) -> Result<(A, HashSet<OwnedKey>), Error> {
        let mut record = Record {
            cache: self as *const Self as usize,
            records: HashSet::new(),
        };

        let asset = if S::_support_hot_reloading::<Private>(&self.source) {
            RECORDING.with(|rec| {
                let old_rec = rec.replace(Some(NonNull::from(&mut record)));
                let result = A::load(self, id);
                rec.set(old_rec);
                result
            })
        } else {
            A::load(self, id)
        };

        Ok((asset?, record.records))
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn add_record<K: Into<OwnedKey>>(&self, key: K) {
        if S::_support_hot_reloading::<Private>(&self.source) {
            RECORDING.with(|rec| {
                if let Some(mut recorder) = rec.get() {
                    let recorder = unsafe { recorder.as_mut() };
                    if recorder.cache == self as *const Self as usize {
                        recorder.records.insert(key.into());
                    }
                }
            });
        }
    }

    /// Temporarily disable dependencies recording.
    ///
    /// This function enables to explicitly disable dependencies recording in
    /// [`Compound::load`]. Assets loaded during the given closure will not be
    /// recorded as dependencies and the currently loading asset will not be
    /// reloaded when they are.
    ///
    /// When hot-reloading is disabled or if the cache's [`Source`] does not
    /// support hot-reloading, this function only returns the result of the
    /// closure given as parameter.
    #[inline]
    pub fn no_record<T, F: FnOnce() -> T>(&self, f: F) -> T {
        #[cfg(feature = "hot-reloading")]
        if S::_support_hot_reloading::<Private>(&self.source) {
            RECORDING.with(|rec| {
                let old_rec = rec.replace(None);
                let result = f();
                rec.set(old_rec);
                result
            })
        } else {
            f()
        }

        #[cfg(not(feature = "hot-reloading"))]
        { f() }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub(crate) fn is_recording(&self) -> bool {
        RECORDING.with(|rec| rec.get().is_some())
    }

    /// Adds an asset to the cache.
    #[cold]
    fn add_asset<A: Compound>(&self, id: &str) -> Result<Handle<A>, Error> {
        let asset = A::_load::<S, Private>(self, id)?;

        let key = OwnedKey::new::<A>(id.into());
        let mut assets = self.assets.write();

        let entry = assets.entry(key).or_insert_with(|| CacheEntry::new(asset, id.into()));

        unsafe { Ok(entry.get_ref()) }
    }

    /// Adds a directory to the cache.
    #[cold]
    fn add_dir<A: Asset>(&self, id: &str) -> Result<DirReader<A, S>, io::Error> {
        #[cfg(feature = "hot-reloading")]
        self.source._add_dir::<A, Private>(id);

        let dir = self.no_record(|| CachedDir::load::<A, S>(self, id))?;

        let key = OwnedKey::new::<A>(id.into());
        let mut dirs = self.dirs.write();

        let dir = dirs.entry(key).or_insert(dir);

        unsafe { Ok(dir.read(self)) }
    }

    /// Loads an asset.
    ///
    /// If the asset is not found in the cache, it is loaded from the source.
    ///
    /// # Errors
    ///
    /// Errors can occur in several cases :
    /// - The asset could not be loaded from the filesystem
    /// - Loaded data could not be converted properly
    /// - The asset has no extension
    #[inline]
    pub fn load<A: Compound>(&self, id: &str) -> Result<Handle<A>, Error> {
        match self.load_cached(id) {
            Some(asset) => Ok(asset),
            None => self.add_asset(id),
        }
    }

    /// Loads an asset from the cache.
    ///
    /// This function does not attempt to load the asset from the source if it
    /// is not found in the cache.
    #[inline]
    pub fn load_cached<A: Compound>(&self, id: &str) -> Option<Handle<A>> {
        let key: &dyn Key = &Key::new::<A>(id);
        let cache = self.assets.read();

        #[cfg(not(feature = "hot-reloading"))]
        let asset = cache.get(key)?;

        #[cfg(feature = "hot-reloading")]
        let asset = match cache.get_key_value(key) {
            Some((key, asset)) => {
                self.add_record(key);
                asset
            },
            None => {
                let key = Key::new::<A>(id);
                self.add_record(key);
                return None;
            },
        };

        Some(unsafe { asset.get_ref() })
    }

    /// Returns `true` if the cache contains the specified asset.
    #[inline]
    pub fn contains<A: Compound>(&self, id: &str) -> bool {
        let key: &dyn Key = &Key::new::<A>(id);
        let cache = self.assets.read();
        cache.contains_key(key)
    }

    /// Loads an asset and panic if an error happens.
    ///
    /// # Panics
    ///
    /// Panics if an error happens while loading the asset (see [`load`]).
    ///
    /// [`load`]: `Self::load`
    #[inline]
    #[track_caller]
    pub fn load_expect<A: Compound>(&self, id: &str) -> Handle<A> {
        self.load(id).unwrap_or_else(|err| {
            panic!("Failed to load essential asset \"{}\": {}", id, err)
        })
    }

    /// Loads all assets of a given type in a directory.
    ///
    /// The directory's id is constructed the same way as assets. To specify
    /// the cache's root, give the empty string (`""`) as id.
    ///
    /// The returned structure can be iterated on to get the loaded assets.
    ///
    /// # Errors
    ///
    /// An error is returned if the given id does not match a valid readable
    /// directory.
    #[inline]
    pub fn load_dir<A: Asset>(&self, id: &str) -> io::Result<DirReader<A, S>> {
        match self.load_cached_dir(id) {
            Some(dir) => Ok(dir),
            None => self.add_dir(id),
        }
    }

    /// Loads an directory from the cache.
    ///
    /// This function does not attempt to load the asset from the source if it
    /// is not found in the cache.
    #[inline]
    pub fn load_cached_dir<A: Asset>(&self, id: &str) -> Option<DirReader<A, S>> {
        let key: &dyn Key = &Key::new::<A>(id);
        let dirs = self.dirs.read();
        dirs.get(key).map(|dir| unsafe { dir.read(self) })
    }

    /// Returns `true` if the cache contains the specified directory.
    #[inline]
    pub fn contains_dir<A: Asset>(&self, id: &str) -> bool {
        let key: &dyn Key = &Key::new::<A>(id);
        let dirs = self.dirs.read();
        dirs.contains_key(key)
    }

    /// Loads an owned version of an asset
    ///
    /// Note that the asset will not be fetched from the cache nor will it be
    /// cached. In addition, hot-reloading does not affect the returned value
    /// (if used during [`Compound::load`], it will still be registered as a
    /// dependency).
    ///
    /// This can be useful if you need ownership on a non-clonable value.
    #[inline]
    pub fn load_owned<A: Compound>(&self, id: &str) -> Result<A, Error> {
        #[cfg(feature = "hot-reloading")]
        if self.is_recording() {
            let key = Key::new::<A>(id);
            self.add_record(key);
            return A::_load::<S, Private>(self, id)
        }

        A::load(self, id)
    }

    /// Removes an asset from the cache, and returns whether it was present in
    /// the cache.
    ///
    /// Note that you need a mutable reference to the cache, so you cannot have
    /// any [`Handle`], [`AssetGuard`], etc when you call this function.
    #[inline]
    pub fn remove<A: Compound>(&mut self, id: &str) -> bool {
        let key: &dyn Key = &Key::new::<A>(id);
        let cache = self.assets.get_mut();
        cache.remove(key).is_some()
    }

    /// Takes ownership on a cached asset.
    ///
    /// The corresponding asset is removed from the cache.
    pub fn take<A: Compound>(&mut self, id: &str) -> Option<A> {
        let key: &dyn Key = &Key::new::<A>(id);
        let cache = self.assets.get_mut();
        cache.remove(key).map(|entry| unsafe { entry.into_inner() })
    }

    /// Clears the cache.
    ///
    /// Removes all cached assets and directories.
    #[inline]
    pub fn clear(&mut self) {
        self.assets.get_mut().clear();
        self.dirs.get_mut().clear();

        #[cfg(feature = "hot-reloading")]
        self.source._clear::<Private>();
    }
}

impl AssetCache<FileSystem> {
    /// Reloads changed assets.
    ///
    /// This function is typically called within a loop.
    ///
    /// If an error occurs while reloading an asset, a warning will be logged
    /// and the asset will be left unchanged.
    ///
    /// This function blocks the current thread until all changed assets are
    /// reloaded, but it does not perform any I/O. However, it needs to lock
    /// some assets for writing, so you **must not** have any [`AssetGuard`]
    /// from the given `AssetCache`, or you might experience deadlocks. You are
    /// free to keep [`Handle`]s, though. The same restriction applies to
    /// [`ReadDir`] and [`ReadAllDir`].
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    pub fn hot_reload(&self) {
        if let Some(reloader) = &self.source.reloader {
            reloader.reload(self);
        }
    }

    /// Enhances hot-reloading.
    ///
    /// Having a `'static` reference to the cache enables some optimizations,
    /// which you can take advantage of with this function. If an `AssetCache`
    /// is behind a `'static` reference, you should always prefer using this
    /// function over [`hot_reload`](`Self::hot_reload`).
    ///
    /// You only have to call this function once for it to take effect. After
    /// calling this function, subsequent calls to `hot_reload` and to this
    /// function have no effect.
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    pub fn enhance_hot_reloading(&'static self) {
        if let Some(reloader) = &self.source.reloader {
            reloader.send_static(self);
        }
    }
}

impl<S> fmt::Debug for AssetCache<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("assets", &self.assets.read())
            .field("dirs", &self.dirs.read())
            .finish()
    }
}

#[inline]
fn load_single<A: Asset, S: Source>(source: &S, id: &str, ext: &str) -> Result<A, Error> {
    let content = source.read(id, ext)?;
    let asset = A::Loader::load(content, ext)?;
    Ok(asset)
}

pub(crate) fn load_from_source<A: Asset, S: Source>(source: &S, id: &str) -> Result<A, Error> {
    let mut error = Error::NoDefaultValue;

    for ext in A::EXTENSIONS {
        match load_single(source, id, ext) {
            Err(err) => error = err.or(error),
            asset => return asset,
        }
    }

    A::default_value(id, error)
}
