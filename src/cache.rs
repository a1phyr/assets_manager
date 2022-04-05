//! Definition of the cache

use crate::{
    anycache::{CacheExt, CacheWithSourceExt, RawCache, RawCacheWithSource},
    asset::{DirLoadable, Storable},
    dirs::DirHandle,
    entry::{CacheEntry, UntypedHandle},
    error::ErrorKind,
    loader::Loader,
    source::{FileSystem, Source},
    utils::{BorrowedKey, HashMap, Key, OwnedKey, RandomState, RwLock},
    AnyCache, Asset, Compound, Error, Handle, SharedString,
};

#[cfg(doc)]
use crate::AssetGuard;

use std::{any::TypeId, fmt, io, path::Path};

#[cfg(feature = "hot-reloading")]
use crate::{
    hot_reloading::{records, HotReloader},
    key::AnyAsset,
};

// Make shards go to different cache lines to reduce contention
#[repr(align(64))]
struct Shard(RwLock<HashMap<OwnedKey, CacheEntry>>);

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
    fn new(min_shards: usize) -> AssetMap {
        let shards = min_shards.next_power_of_two();

        let hash_builder = RandomState::new();
        let shards = (0..shards)
            .map(|_| Shard(RwLock::new(HashMap::with_hasher(hash_builder.clone()))))
            .collect();

        AssetMap {
            hash_builder,
            shards,
        }
    }

    fn get_shard(&self, key: BorrowedKey) -> &Shard {
        use std::hash::*;

        let mut hasher = self.hash_builder.build_hasher();
        key.hash(&mut hasher);
        let id = (hasher.finish() as usize) & (self.shards.len() - 1);
        &self.shards[id]
    }

    fn get_shard_mut(&mut self, key: BorrowedKey) -> &mut Shard {
        use std::hash::*;

        let mut hasher = self.hash_builder.build_hasher();
        key.hash(&mut hasher);
        let id = (hasher.finish() as usize) & (self.shards.len() - 1);
        &mut self.shards[id]
    }

    pub fn get_entry(&self, id: &str, type_id: TypeId) -> Option<UntypedHandle> {
        let key = BorrowedKey::new_with(id, type_id);
        let shard = self.get_shard(key).0.read();
        let entry = shard.get(&key as &dyn Key)?;
        unsafe { Some(entry.inner().extend_lifetime()) }
    }

    #[cfg(feature = "hot-reloading")]
    pub fn get_key_entry(&self, id: &str, type_id: TypeId) -> Option<(OwnedKey, UntypedHandle)> {
        let key = BorrowedKey::new_with(id, type_id);
        let shard = self.get_shard(key).0.read();
        let (key, entry) = shard.get_key_value(&key as &dyn Key)?;
        unsafe { Some((key.clone(), entry.inner().extend_lifetime())) }
    }

    pub fn insert(&self, id: SharedString, type_id: TypeId, entry: CacheEntry) -> UntypedHandle {
        let key = OwnedKey::new_with(id, type_id);
        let shard = &mut *self.get_shard(key.borrow()).0.write();
        let entry = shard.entry(key).or_insert(entry);
        unsafe { entry.inner().extend_lifetime() }
    }

    #[cfg(feature = "hot-reloading")]
    pub fn update_or_insert(&self, id: SharedString, type_id: TypeId, value: Box<dyn AnyAsset>) {
        use std::collections::hash_map::Entry;

        let key = OwnedKey::new_with(id, type_id);
        let shard = &mut *self.get_shard(key.borrow()).0.write();

        match shard.entry(key) {
            Entry::Occupied(entry) => value.reload(entry.get().inner()),
            Entry::Vacant(entry) => {
                let id = entry.key().clone().into_id();
                entry.insert(value.create(id));
            }
        }
    }

    pub fn contains_key(&self, id: &str, type_id: TypeId) -> bool {
        let key = BorrowedKey::new_with(id, type_id);
        let shard = self.get_shard(key).0.read();
        shard.contains_key(&key as &dyn Key)
    }

    fn take(&mut self, id: &str, type_id: TypeId) -> Option<CacheEntry> {
        let key = BorrowedKey::new_with(id, type_id);
        self.get_shard_mut(key).0.get_mut().remove(&key as &dyn Key)
    }

    #[inline]
    fn remove(&mut self, id: &str, type_id: TypeId) -> bool {
        self.take(id, type_id).is_some()
    }

    fn clear(&mut self) {
        for shard in &mut *self.shards {
            shard.0.get_mut().clear();
        }
    }
}

impl fmt::Debug for AssetMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();

        for shard in &*self.shards {
            map.entries(&**shard.0.read());
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
pub struct AssetCache<S = FileSystem> {
    #[cfg(feature = "hot-reloading")]
    pub(crate) reloader: Option<HotReloader>,

    pub(crate) assets: AssetMap,
    source: S,
}

impl<S> RawCache for AssetCache<S> {
    #[inline]
    fn assets(&self) -> &crate::cache::AssetMap {
        &self.assets
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    fn reloader(&self) -> Option<&HotReloader> {
        self.reloader.as_ref()
    }
}

impl<S: Source> RawCacheWithSource for AssetCache<S> {
    type Source = S;

    #[inline]
    fn get_source(&self) -> &S {
        &self.source
    }
}

impl AssetCache<FileSystem> {
    /// Creates a cache that loads assets from the given directory.
    ///
    /// # Errors
    ///
    /// An error will be returned if `path` is not valid readable directory.
    #[inline]
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<AssetCache<FileSystem>> {
        let source = FileSystem::new(path)?;
        Ok(Self::with_source(source))
    }
}

impl<S: Source> AssetCache<S> {
    /// Creates a cache that loads assets from the given source and tries to
    /// start hot-reloading (if feature `hot-reloading` is used).
    ///
    /// If hot-reloading fails to start, an error is logged.
    pub fn with_source(source: S) -> AssetCache<S> {
        Self {
            #[cfg(feature = "hot-reloading")]
            reloader: HotReloader::make(&source),

            assets: AssetMap::new(32),
            source,
        }
    }
}

impl<S> AssetCache<S> {
    /// Creates a cache that loads assets from the given source.
    pub fn without_hot_reloading(source: S) -> AssetCache<S> {
        Self {
            #[cfg(feature = "hot-reloading")]
            reloader: None,

            assets: AssetMap::new(32),
            source,
        }
    }

    /// Returns a reference to the cache's [`Source`].
    #[inline]
    pub fn source(&self) -> &S {
        &self.source
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

    /// Gets a value from the cache.
    ///
    /// The value does not have to be an asset, but if it is not, its type must
    /// be marked with the [`Storable`] trait.
    ///
    /// This function does not attempt to load the value from the source if it
    /// is not found in the cache.
    #[inline]
    pub fn get_cached<A: Storable>(&self, id: &str) -> Option<Handle<A>> {
        self._get_cached(id)
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// As for `get_cached`, non-assets types must be marked with [`Storable`].
    #[inline]
    pub fn get_or_insert<A: Storable>(&self, id: &str, default: A) -> Handle<A> {
        self._get_or_insert(id, default)
    }

    /// Returns `true` if the cache contains the specified asset.
    #[inline]
    pub fn contains<A: Storable>(&self, id: &str) -> bool {
        self.assets.contains_key(id, TypeId::of::<A>())
    }

    /// Removes an asset from the cache, and returns whether it was present in
    /// the cache.
    ///
    /// Note that you need a mutable reference to the cache, so you cannot have
    /// any [`Handle`], [`AssetGuard`], etc when you call this function.
    #[inline]
    pub fn remove<A: Storable>(&mut self, id: &str) -> bool {
        let removed = self.assets.remove(id, TypeId::of::<A>());

        #[cfg(feature = "hot-reloading")]
        if let Some(reloader) = &self.reloader {
            if A::HOT_RELOADED && removed {
                reloader.remove_asset::<A>(SharedString::from(id));
            }
        }

        removed
    }

    /// Takes ownership on a cached asset.
    ///
    /// The corresponding asset is removed from the cache.
    #[inline]
    pub fn take<A: Storable>(&mut self, id: &str) -> Option<A> {
        self.assets.take(id, TypeId::of::<A>()).map(|e| {
            let (asset, _id) = e.into_inner();

            #[cfg(feature = "hot-reloading")]
            if let Some(reloader) = &self.reloader {
                if A::HOT_RELOADED {
                    reloader.remove_asset::<A>(_id);
                }
            }

            asset
        })
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

impl<S> AssetCache<S>
where
    S: Source,
{
    /// Converts to an `AnyCache`.
    #[inline]
    pub fn as_any_cache(&self) -> AnyCache {
        self._as_any_cache()
    }

    /// Loads an asset.
    ///
    /// If the asset is not found in the cache, it is loaded from the source.
    ///
    /// # Errors
    ///
    /// Errors for `Asset`s can occur in several cases :
    /// - The source could not be read
    /// - Loaded data could not be converted properly
    /// - The asset has no extension
    #[inline]
    pub fn load<A: Compound>(&self, id: &str) -> Result<Handle<A>, Error> {
        self._load(id)
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
        self._load_expect(id)
    }

    /// Loads an directory from the cache.
    ///
    /// This function does not attempt to load the it from the source if it is
    /// not found in the cache.
    #[inline]
    pub fn get_cached_dir<A: DirLoadable>(
        &self,
        id: &str,
        recursive: bool,
    ) -> Option<DirHandle<A>> {
        self._get_cached_dir(id, recursive)
    }

    /// Returns `true` if the cache contains the specified directory with the
    /// given `recursive` parameter.
    #[inline]
    pub fn contains_dir<A: DirLoadable>(&self, id: &str, recursive: bool) -> bool {
        self._contains_dir::<A>(id, recursive)
    }

    /// Loads all assets of a given type from a directory.
    ///
    /// If `recursive` is `true`, this function also loads assets recursively
    /// from subdirectories.
    ///
    /// The directory's id is constructed the same way as assets. To specify
    /// the cache's root, give the empty string (`""`) as id.
    ///
    /// The returned structure can be use to iterate over the loaded assets.
    ///
    /// # Errors
    ///
    /// An error is returned if the given id does not match a valid readable
    /// directory.
    ///
    /// When loading a directory recursively, directories that can't be read are
    /// ignored.
    #[inline]
    pub fn load_dir<A: DirLoadable>(
        &self,
        id: &str,
        recursive: bool,
    ) -> Result<DirHandle<A>, Error> {
        self._load_dir(id, recursive)
    }

    /// Loads an owned version of an asset
    ///
    /// Note that the asset will not be fetched from the cache nor will it be
    /// cached. In addition, hot-reloading does not affect the returned value
    /// (if used during [`Compound::load`]. It will still be registered as a
    /// dependency).
    ///
    /// This can be useful if you need ownership on a non-clonable value.
    #[inline]
    pub fn load_owned<A: Compound>(&self, id: &str) -> Result<A, Error> {
        self._load_owned(id)
    }
}

impl<S> AssetCache<S>
where
    S: Source + Sync,
{
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
    /// free to keep [`Handle`]s, though.
    ///
    /// If `self.source()` was created without hot-reloading or if it failed to
    /// start, this function is a no-op.
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    #[inline]
    pub fn hot_reload(&self) {
        if let Some(reloader) = &self.reloader {
            reloader.reload(&self.assets);
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
            reloader.send_static(&self.assets);
        }
    }
}

impl<S> fmt::Debug for AssetCache<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("assets", &self.assets)
            .finish()
    }
}

#[inline]
fn load_single<A, S>(source: &S, id: &str, ext: &str) -> Result<A, ErrorKind>
where
    A: Asset,
    S: Source + ?Sized,
{
    let content = source.read(id, ext)?;
    let asset = A::Loader::load(content, ext)?;
    Ok(asset)
}

pub(crate) fn load_from_source<A, S>(source: &S, id: &str) -> Result<A, Error>
where
    A: Asset,
    S: Source + ?Sized,
{
    let mut error = ErrorKind::NoDefaultValue;

    for ext in A::EXTENSIONS {
        match load_single(source, id, ext) {
            Err(err) => error = err.or(error),
            Ok(asset) => return Ok(asset),
        }
    }

    A::default_value(id, Error::from_kind(id.into(), error))
}
