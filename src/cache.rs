//! Definition of the cache

use crate::{
    Compound, Error, Handle, SharedString,
    asset::{DirLoadable, Storable},
    entry::{CacheEntry, UntypedHandle},
    key::Type,
    source::{FileSystem, Source},
    utils::{RandomState, RwLock},
};

#[cfg(doc)]
use crate::AssetReadGuard;

use std::{any::TypeId, fmt, io, path::Path};

#[cfg(feature = "hot-reloading")]
use crate::hot_reloading::{HotReloader, records};

// Make shards go to different cache lines to reduce contention
#[repr(align(64))]
struct Shard(RwLock<crate::map::AssetMap>);

/// A map to store assets, optimized for concurrency.
///
/// This type has several uses:
/// - Provide a safe wrapper to ensure that no issue with lifetimes happen.
/// - Make a sharded lock map to reduce contention on the `RwLock` that guard
///   inner `HashMap`s.
/// - Provide an interface with the minimum of generics to reduce compile times.
pub(crate) struct AssetMap {
    hash_builder: RandomState,
    shards: Box<[Shard]>,
}

impl AssetMap {
    fn new() -> AssetMap {
        let shards = match std::thread::available_parallelism() {
            Ok(n) => 4 * n.get().next_power_of_two(),
            Err(err) => {
                log::error!("Failed to get available parallelism: {err}");
                32
            }
        };

        let hash_builder = RandomState::default();
        let shards = (0..shards)
            .map(|_| Shard(RwLock::new(crate::map::AssetMap::new())))
            .collect();

        AssetMap {
            hash_builder,
            shards,
        }
    }

    fn hash_one(&self, key: (TypeId, &str)) -> u64 {
        std::hash::BuildHasher::hash_one(&self.hash_builder, key)
    }

    fn get_shard(&self, hash: u64) -> &Shard {
        let id = (hash as usize) & (self.shards.len() - 1);
        &self.shards[id]
    }

    fn get_shard_mut(&mut self, hash: u64) -> &mut Shard {
        let id = (hash as usize) & (self.shards.len() - 1);
        &mut self.shards[id]
    }

    fn take(&mut self, id: &str, type_id: TypeId) -> Option<CacheEntry> {
        let hash = self.hash_one((type_id, id));
        self.get_shard_mut(hash).0.get_mut().take(hash, id, type_id)
    }

    fn remove(&mut self, id: &str, type_id: TypeId) -> bool {
        self.take(id, type_id).is_some()
    }

    fn clear(&mut self) {
        for shard in &mut *self.shards {
            shard.0.get_mut().clear();
        }
    }

    fn get(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        let hash = self.hash_one((type_id, id));
        let shard = self.get_shard(hash).0.read();
        let entry = shard.get(hash, id, type_id)?;
        unsafe { Some(entry.extend_lifetime()) }
    }

    fn insert(&self, entry: CacheEntry) -> &UntypedHandle {
        let hash = self.hash_one(entry.as_key());
        let shard = &mut *self.get_shard(hash).0.write();
        let entry = shard.insert(hash, entry, &self.hash_builder);
        unsafe { entry.extend_lifetime() }
    }

    fn contains_key(&self, id: &str, type_id: TypeId) -> bool {
        let hash = self.hash_one((type_id, id));
        let shard = self.get_shard(hash).0.read();
        shard.get(hash, id, type_id).is_some()
    }
}

impl fmt::Debug for AssetMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();

        for shard in &*self.shards {
            map.entries(shard.0.read().iter_for_debug());
        }

        map.finish()
    }
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
/// #   #[cfg(feature = "hot-reloading")]
///     cache.hot_reload();
///     println!("Position: {:?}", point_handle.read());
/// #   break;
/// }
///
/// # }}
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct AssetCache {
    #[cfg(feature = "hot-reloading")]
    pub(crate) reloader: Option<HotReloader>,

    pub(crate) assets: AssetMap,
    source: Box<dyn Source + Send + Sync>,
}

impl AssetCache {
    /// Creates a cache that loads assets from the given directory.
    ///
    /// # Errors
    ///
    /// An error will be returned if `path` is not valid readable directory.
    #[inline]
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let source = FileSystem::new(path)?;
        Ok(Self::with_source(source))
    }

    /// Creates a cache that loads assets from the given source and tries to
    /// start hot-reloading (if feature `hot-reloading` is used).
    ///
    /// If hot-reloading fails to start, an error is logged.
    pub fn with_source<S: Source + Send + Sync + 'static>(source: S) -> Self {
        Self {
            #[cfg(feature = "hot-reloading")]
            reloader: HotReloader::start(&source),

            assets: AssetMap::new(),
            source: Box::new(source),
        }
    }

    /// Creates a cache that loads assets from the given source.
    pub fn without_hot_reloading<S: Source + Send + Sync + 'static>(source: S) -> Self {
        Self {
            #[cfg(feature = "hot-reloading")]
            reloader: None,

            assets: AssetMap::new(),
            source: Box::new(source),
        }
    }

    /// Returns a reference to the cache's [`Source`].
    #[inline]
    pub fn source(&self) -> impl Source + Send + Sync + '_ {
        #[cfg(feature = "hot-reloading")]
        return CacheSource { cache: self };

        #[cfg(not(feature = "hot-reloading"))]
        &*self.source
    }

    /// Returns a reference to the cache's [`Source`].
    #[inline]
    #[deprecated = "use `.source()` instead"]
    pub fn raw_source(&self) -> impl Source + Send + Sync + '_ {
        self.source()
    }

    /// Returns a reference to the cache's [`Source`].
    #[inline]
    pub fn downcast_raw_source<S: Source + 'static>(&self) -> Option<&S> {
        self.source.downcast_ref()
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub(crate) fn reloader(&self) -> Option<&HotReloader> {
        self.reloader.as_ref()
    }

    /// Temporarily prevent `Compound` dependencies to be recorded.
    ///
    /// This function disables dependencies recording in [`Compound::load`].
    /// Assets loaded during the given closure will not be recorded as
    /// dependencies and the currently loading asset will not be reloaded when
    /// they are.
    ///
    /// When hot-reloading is disabled or if the cache's [`Source`] does not
    /// support hot-reloading, this function only returns the result of the
    /// closure given as parameter.
    #[inline]
    pub fn no_record<T, F: FnOnce() -> T>(&self, f: F) -> T {
        #[cfg(feature = "hot-reloading")]
        {
            records::no_record(f)
        }

        #[cfg(not(feature = "hot-reloading"))]
        {
            f()
        }
    }

    #[cfg(feature = "hot-reloading")]
    fn add_record(&self, handle: &UntypedHandle) {
        if let Some(reloader) = &self.reloader {
            if let Some(typ) = handle.typ() {
                let key = crate::key::AssetKey::new(handle.id().clone(), typ);
                records::add_record(reloader, key);
            }
        }
    }

    /// Loads an asset.
    ///
    /// If the asset is not found in the cache, it is loaded from the source.
    ///
    /// # Errors
    ///
    /// Errors for `Asset`s can occur in several cases:
    /// - The source could not be read
    /// - Loaded data could not be converted properly
    /// - The asset has no extension
    #[inline]
    pub fn load<T: Compound>(&self, id: &str) -> Result<&Handle<T>, Error> {
        let handle = self.load_entry(id, Type::of_asset::<T>())?;
        Ok(handle.downcast_ref_ok())
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
    pub fn load_expect<T: Compound>(&self, id: &str) -> &Handle<T> {
        #[cold]
        #[track_caller]
        fn expect_failed(err: Error) -> ! {
            panic!(
                "Failed to load essential asset \"{}\": {}",
                err.id(),
                err.reason()
            )
        }

        match self.load(id) {
            Ok(h) => h,
            Err(err) => expect_failed(err),
        }
    }

    fn load_entry(&self, id: &str, typ: Type) -> Result<&UntypedHandle, Error> {
        let result = match self.assets.get(id, typ.type_id) {
            Some(handle) => Ok(handle),
            None => self.add_asset(id, typ),
        };

        #[cfg(feature = "hot-reloading")]
        if let Ok(handle) = result {
            self.add_record(handle);
        }

        result
    }

    #[cold]
    fn add_asset(&self, id: &str, typ: Type) -> Result<&UntypedHandle, Error> {
        log::trace!("Loading \"{}\"", id);

        let id = SharedString::from(id);
        let handle = crate::asset::load_and_record(self, id, typ)?;

        Ok(self.assets.insert(handle))
    }

    /// Gets a value from the cache.
    ///
    /// This function does not attempt to load the value from the source if it
    /// is not found in the cache.
    #[inline]
    pub fn get_cached<T: Storable>(&self, id: &str) -> Option<&Handle<T>> {
        let handle = self.get_cached_untyped(id, TypeId::of::<T>())?;
        Some(handle.downcast_ref_ok())
    }

    /// Gets a value with the given type from the cache.
    ///
    /// This is an equivalent of `get_cached` but with a dynamic type.
    pub fn get_cached_untyped(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        let result = self.assets.get(id, type_id);

        #[cfg(feature = "hot-reloading")]
        if let Some(handle) = result {
            self.add_record(handle);
        }

        result
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// Assets added via this function will *never* be reloaded.
    #[inline]
    pub fn get_or_insert<T: Storable>(&self, id: &str, default: T) -> &Handle<T> {
        let handle = match self.get_cached_untyped(id, TypeId::of::<T>()) {
            Some(handle) => handle,
            None => self.add_any(id, default),
        };

        handle.downcast_ref_ok()
    }

    #[cold]
    fn add_any<T: Storable>(&self, id: &str, asset: T) -> &UntypedHandle {
        let id = SharedString::from(id);
        let handle = CacheEntry::new_any(asset, id, false);

        self.assets.insert(handle)
    }

    /// Returns `true` if the cache contains the specified asset.
    #[inline]
    pub fn contains<T: Storable>(&self, id: &str) -> bool {
        self.assets.contains_key(id, TypeId::of::<T>())
    }

    /// Loads a directory.
    ///
    /// The directory's id is constructed the same way as assets. To specify
    /// the cache's root, give the empty string (`""`) as id.
    ///
    /// Note that this function only gets the ids of assets, and that are not
    /// actually loaded. The returned handle can be use to iterate over them.
    ///
    /// # Errors
    ///
    /// An error is returned if the given id does not match a valid readable
    /// directory.
    #[inline]
    pub fn load_dir<T: DirLoadable>(
        &self,
        id: &str,
    ) -> Result<&Handle<crate::Directory<T>>, Error> {
        self.load::<crate::Directory<T>>(id)
    }

    /// Loads a directory and its subdirectories.
    ///
    /// The directory's id is constructed the same way as assets. To specify
    /// the cache's root, give the empty string (`""`) as id.
    ///
    /// Note that this function only gets the ids of assets, and that are not
    /// actually loaded. The returned handle can be use to iterate over them.
    ///
    /// # Errors
    ///
    /// An error is returned if the given id does not match a valid readable
    /// directory.
    ///
    /// When loading a directory recursively, directories that can't be read are
    /// ignored.
    #[inline]
    pub fn load_rec_dir<T: DirLoadable>(
        &self,
        id: &str,
    ) -> Result<&Handle<crate::RecursiveDirectory<T>>, Error> {
        self.load::<crate::RecursiveDirectory<T>>(id)
    }

    /// Loads an owned version of an asset.
    ///
    /// Note that the asset will not be fetched from the cache nor will it be
    /// cached. In addition, hot-reloading does not affect the returned value.
    ///
    /// This can be useful if you need ownership on a non-clonable value.
    ///
    /// Inside an implementation [`Compound::load`], you should use `T::load`
    /// directly.
    #[inline]
    pub fn load_owned<T: Compound>(&self, id: &str) -> Result<T, Error> {
        let id = SharedString::from(id);
        T::load(self, &id).map_err(|err| Error::new(id, err))
    }
}

impl AssetCache {
    /// Removes an asset from the cache, and returns whether it was present in
    /// the cache.
    ///
    /// Note that you need a mutable reference to the cache, so you cannot have
    /// any [`Handle`], [`AssetReadGuard`], etc when you call this function.
    #[inline]
    pub fn remove<T: Storable>(&mut self, id: &str) -> bool {
        self.assets.remove(id, TypeId::of::<T>())
    }

    /// Takes ownership on a cached asset.
    ///
    /// The corresponding asset is removed from the cache.
    #[inline]
    pub fn take<T: Storable>(&mut self, id: &str) -> Option<T> {
        let (asset, _) = self.assets.take(id, TypeId::of::<T>())?.into_inner();
        Some(asset)
    }

    /// Clears the cache.
    ///
    /// Removes all cached assets and directories.
    #[inline]
    pub fn clear(&mut self) {
        self.assets.clear();

        #[cfg(feature = "hot-reloading")]
        if let Some(reloader) = &self.reloader {
            reloader.clear();
        }
    }
}

impl AssetCache {
    /// Reloads changed assets.
    ///
    /// This function is typically called within a loop.
    ///
    /// If an error occurs while reloading an asset, a warning will be logged
    /// and the asset will be left unchanged.
    ///
    /// This function blocks the current thread until all changed assets are
    /// reloaded, but it does not perform any I/O. However, it needs to lock
    /// some assets for writing, so you **must not** have any [`AssetReadGuard`]
    /// from the given `AssetCache`, or you might experience deadlocks. You are
    /// free to keep [`Handle`]s, though.
    ///
    /// If `self.source()` was created without hot-reloading or if it failed to
    /// start, this function is a no-op.
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    #[inline]
    pub fn hot_reload(&self) {
        if let Some(reloader) = &self.reloader {
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
    ///
    /// If `self.source()` was created without hot-reloading or if it failed to
    /// start, this function is a no-op.
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    #[inline]
    pub fn enhance_hot_reloading(&'static self) {
        if let Some(reloader) = &self.reloader {
            reloader.send_static(self);
        }
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn reload_untyped(
        &self,
        id: &SharedString,
        typ: Type,
    ) -> Option<records::Dependencies> {
        let handle = self.get_cached_untyped(id, typ.type_id)?;

        let load_asset = || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (typ.inner.load)(self, id.clone())
            }))
        };
        let (entry, deps) = if let Some(reloader) = &self.reloader {
            records::record(reloader, load_asset)
        } else {
            log::warn!("No reloader in hot-reloading context");
            (load_asset(), records::Dependencies::new())
        };

        match entry {
            Ok(Ok(e)) => {
                handle.write(e);
                log::info!("Reloading \"{id}\"");
                Some(deps)
            }
            Ok(Err(err)) => {
                log::warn!("Error reloading \"{id}\": {}", err.reason());
                None
            }
            Err(_) => {
                log::warn!("Panic while reloading \"{id}\"");
                None
            }
        }
    }

    /// Returns `true` if values stored in this cache may be hot-reloaded.
    #[inline]
    pub fn is_hot_reloaded(&self) -> bool {
        #[cfg(feature = "hot-reloading")]
        return self.reloader.is_some();

        #[cfg(not(feature = "hot-reloading"))]
        false
    }
}

impl fmt::Debug for AssetCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("assets", &self.assets)
            .finish()
    }
}

#[cfg(feature = "hot-reloading")]
struct CacheSource<'a> {
    cache: &'a AssetCache,
}

#[cfg(feature = "hot-reloading")]
impl Source for CacheSource<'_> {
    fn read(&self, id: &str, ext: &str) -> io::Result<crate::source::FileContent> {
        if let Some(reloader) = &self.cache.reloader {
            records::add_file_record(reloader, id, ext);
        }
        self.cache.source.read(id, ext)
    }

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(crate::source::DirEntry)) -> io::Result<()> {
        if let Some(reloader) = &self.cache.reloader {
            records::add_dir_record(reloader, id);
        }
        self.cache.source.read_dir(id, f)
    }

    fn exists(&self, entry: crate::source::DirEntry) -> bool {
        self.cache.source.exists(entry)
    }
}
