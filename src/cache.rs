//! Definition of the cache

use crate::{
    asset::{DirLoadable, Storable},
    dirs::DirHandle,
    entry::{CacheEntry, CacheEntryInner},
    error::ErrorKind,
    loader::Loader,
    source::{FileSystem, Source},
    utils::{BorrowedKey, HashMap, Key, OwnedKey, Private, RandomState, RwLock},
    Asset, Compound, Error, Handle, SharedString,
};

#[cfg(doc)]
use crate::AssetGuard;

use std::{any::TypeId, fmt, io, path::Path};

#[cfg(feature = "hot-reloading")]
use crate::{hot_reloading::HotReloader, utils::HashSet};

#[cfg(feature = "hot-reloading")]
use std::{cell::Cell, ptr::NonNull};

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
pub(crate) struct Map {
    hash_builder: RandomState,
    shards: Box<[Shard]>,
}

impl Map {
    fn new(min_shards: usize) -> Map {
        let shards = min_shards.next_power_of_two();

        let hash_builder = RandomState::new();
        let shards = (0..shards)
            .map(|_| Shard(RwLock::new(HashMap::with_hasher(hash_builder.clone()))))
            .collect();

        Map {
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

    pub fn get_entry(&self, key: BorrowedKey) -> Option<CacheEntryInner> {
        let shard = self.get_shard(key).0.read();
        let entry = shard.get(&key as &dyn Key)?;
        unsafe { Some(entry.inner().extend_lifetime()) }
    }

    #[cfg(feature = "hot-reloading")]
    pub fn get_key_entry(&self, key: BorrowedKey) -> Option<(OwnedKey, CacheEntryInner)> {
        let shard = self.get_shard(key).0.read();
        let (key, entry) = shard.get_key_value(&key as &dyn Key)?;
        unsafe { Some((key.clone(), entry.inner().extend_lifetime())) }
    }

    fn insert(&self, key: OwnedKey, entry: CacheEntry) -> CacheEntryInner {
        let shard = &mut *self.get_shard(key.borrow()).0.write();
        let entry = shard.entry(key).or_insert(entry);
        unsafe { entry.inner().extend_lifetime() }
    }

    #[cfg(feature = "hot-reloading")]
    pub fn update_or_insert<T>(
        &self,
        key: OwnedKey,
        val: T,
        on_occupied: impl FnOnce(T, &CacheEntry),
        on_vacant: impl FnOnce(T, SharedString) -> CacheEntry,
    ) {
        use std::collections::hash_map::Entry;
        let shard = &mut *self.get_shard(key.borrow()).0.write();

        match shard.entry(key) {
            Entry::Occupied(entry) => on_occupied(val, entry.get()),
            Entry::Vacant(entry) => {
                let id = entry.key().clone().into_id();
                entry.insert(on_vacant(val, id));
            }
        }
    }

    fn contains_key(&self, key: BorrowedKey) -> bool {
        let shard = self.get_shard(key).0.read();
        shard.contains_key(&key as &dyn Key)
    }

    fn take(&mut self, key: BorrowedKey) -> Option<CacheEntry> {
        self.get_shard_mut(key).0.get_mut().remove(&key as &dyn Key)
    }

    #[inline]
    fn remove(&mut self, key: BorrowedKey) -> bool {
        self.take(key).is_some()
    }

    fn clear(&mut self) {
        for shard in &mut *self.shards {
            shard.0.get_mut().clear();
        }
    }
}

impl fmt::Debug for Map {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();

        for shard in &*self.shards {
            map.entries(&**shard.0.read());
        }

        map.finish()
    }
}

#[cfg(feature = "hot-reloading")]
struct Record {
    cache: usize,
    records: HashSet<OwnedKey>,
}

#[cfg(feature = "hot-reloading")]
impl Record {
    fn new(cache: usize) -> Record {
        Record {
            cache,
            records: HashSet::new(),
        }
    }

    fn insert(&mut self, cache: usize, key: OwnedKey) {
        if self.cache == cache {
            self.records.insert(key);
        }
    }
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
/// #   #[cfg(feature = "hot-reloading")]
///     cache.hot_reload();
///     println!("Position: {:?}", point_handle.read());
/// #   break;
/// }
///
/// # }}
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct AssetCache<S: ?Sized = FileSystem> {
    #[cfg(feature = "hot-reloading")]
    pub(crate) reloader: Option<HotReloader>,

    pub(crate) assets: Map,
    source: S,
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
        AssetCache {
            #[cfg(feature = "hot-reloading")]
            reloader: HotReloader::make(&source),

            assets: Map::new(32),
            source,
        }
    }
}

impl<S> AssetCache<S> {
    /// Creates a cache that loads assets from the given source.
    pub fn without_hot_reloading(source: S) -> AssetCache<S> {
        AssetCache {
            #[cfg(feature = "hot-reloading")]
            reloader: None,

            assets: Map::new(32),
            source,
        }
    }
}

impl<S> AssetCache<S>
where
    S: ?Sized,
{
    /// Returns a reference to the cache's [`Source`].
    #[inline]
    pub fn source(&self) -> &S {
        &self.source
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn add_record(&self, key: OwnedKey) {
        RECORDING.with(|rec| {
            if let Some(mut recorder) = rec.get() {
                let recorder = unsafe { recorder.as_mut() };
                recorder.insert(self as *const Self as *const () as usize, key);
            }
        });
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
            RECORDING.with(|rec| {
                let old_rec = rec.replace(None);
                let result = f();
                rec.set(old_rec);
                result
            })
        }

        #[cfg(not(feature = "hot-reloading"))]
        {
            f()
        }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub(crate) fn is_recording(&self) -> bool {
        RECORDING.with(|rec| rec.get().is_some())
    }

    /// Adds an asset to the cache.
    ///
    /// This function does not not have the asset kind as generic parameter to
    /// reduce monomorphisation.
    #[cold]
    fn add_asset(
        &self,
        id: &str,
        type_id: TypeId,
        load: fn(&Self, SharedString) -> Result<CacheEntry, Error>,
    ) -> Result<CacheEntryInner, Error> {
        log::trace!("Loading \"{}\"", id);

        let id = SharedString::from(id);
        let entry = load(self, id.clone())?;
        let key = OwnedKey::new_with(id, type_id);

        Ok(self.assets.insert(key, entry))
    }

    /// Adds any value to the cache.
    #[cold]
    fn add_any<A: Storable>(&self, id: &str, asset: A) -> CacheEntryInner {
        let id = SharedString::from(id);
        let entry = CacheEntry::new(asset, id.clone());
        let key = OwnedKey::new::<A>(id);

        self.assets.insert(key, entry)
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
        Some(self.get_cached_entry::<A>(id)?.handle())
    }

    #[inline]
    fn get_cached_entry<A: Storable>(&self, id: &str) -> Option<CacheEntryInner> {
        self.get_cached_entry_inner(id, TypeId::of::<A>(), A::HOT_RELOADED)
    }

    fn get_cached_entry_inner(
        &self,
        id: &str,
        type_id: TypeId,
        _hot_reloaded: bool,
    ) -> Option<CacheEntryInner> {
        let key = BorrowedKey::new_with(id, type_id);

        #[cfg(not(feature = "hot-reloading"))]
        {
            self.assets.get_entry(key)
        }

        #[cfg(feature = "hot-reloading")]
        if _hot_reloaded {
            let (key, entry) = match self.assets.get_key_entry(key) {
                Some((key, entry)) => (key, Some(entry)),
                None => (key.to_owned(), None),
            };
            self.add_record(key);
            entry
        } else {
            self.assets.get_entry(key)
        }
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// As for `get_cached`, non-assets types must be marked with [`Storable`].
    #[inline]
    pub fn get_or_insert<A: Storable>(&self, id: &str, default: A) -> Handle<A> {
        let entry = match self.get_cached_entry::<A>(id) {
            Some(entry) => entry,
            None => self.add_any(id, default),
        };

        entry.handle()
    }

    /// Returns `true` if the cache contains the specified asset.
    #[inline]
    pub fn contains<A: Storable>(&self, id: &str) -> bool {
        let key = BorrowedKey::new::<A>(id);
        self.assets.contains_key(key)
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
    ) -> Option<DirHandle<A, S>> {
        Some(if recursive {
            let handle = self.get_cached(id)?;
            DirHandle::new_rec(handle, self)
        } else {
            let handle = self.get_cached(id)?;
            DirHandle::new(handle, self)
        })
    }

    /// Returns `true` if the cache contains the specified directory with the
    /// given `recursive` parameter.
    #[inline]
    pub fn contains_dir<A: DirLoadable>(&self, id: &str, recursive: bool) -> bool {
        self.get_cached_dir::<A>(id, recursive).is_some()
    }

    /// Removes an asset from the cache, and returns whether it was present in
    /// the cache.
    ///
    /// Note that you need a mutable reference to the cache, so you cannot have
    /// any [`Handle`], [`AssetGuard`], etc when you call this function.
    #[inline]
    pub fn remove<A: Storable>(&mut self, id: &str) -> bool {
        let key = BorrowedKey::new::<A>(id);
        let removed = self.assets.remove(key);

        #[cfg(feature = "hot-reloading")]
        if let Some(reloader) = &self.reloader {
            if removed {
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
        let key = BorrowedKey::new::<A>(id);
        self.assets.take(key).map(|e| {
            let (asset, _id) = e.into_inner();

            #[cfg(feature = "hot-reloading")]
            if let Some(reloader) = &self.reloader {
                reloader.remove_asset::<A>(_id);
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
    S: Source + ?Sized,
{
    #[cfg(feature = "hot-reloading")]
    pub(crate) fn record_load<A: Compound>(
        &self,
        id: &str,
    ) -> Result<(A, HashSet<OwnedKey>), crate::BoxedError> {
        let mut record = Record::new(self as *const Self as *const () as usize);

        let asset = if self.reloader.is_some() {
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
        let entry = match self.get_cached_entry::<A>(id) {
            Some(entry) => entry,
            None => {
                let load = A::_load_and_record_entry::<S, Private>;
                let type_id = TypeId::of::<A>();
                self.add_asset(id, type_id, load)?
            }
        };

        Ok(entry.handle())
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
        #[cold]
        #[track_caller]
        fn expect_failed(err: Error) -> ! {
            panic!(
                "Failed to load essential asset \"{}\": {}",
                err.id(),
                err.reason()
            )
        }

        // Do not use `unwrap_or_else` as closures do not have #[track_caller]
        match self.load(id) {
            Ok(h) => h,
            Err(err) => expect_failed(err),
        }
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
    ) -> Result<DirHandle<A, S>, Error> {
        Ok(if recursive {
            let handle = self.load(id)?;
            DirHandle::new_rec(handle, self)
        } else {
            let handle = self.load(id)?;
            DirHandle::new(handle, self)
        })
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
        let id = SharedString::from(id);
        let asset = A::_load_and_record::<S, Private>(self, &id);

        #[cfg(feature = "hot-reloading")]
        if A::HOT_RELOADED && self.is_recording() {
            let key = OwnedKey::new::<A>(id);
            self.add_record(key);
        }

        asset
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
}

impl<S> fmt::Debug for AssetCache<S>
where
    S: ?Sized,
{
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
