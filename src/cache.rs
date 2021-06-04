//! Definition of the cache

use crate::{
    Asset, Error, Compound, Handle,
    asset::{DirLoadable, Storable},
    dirs::DirHandle,
    entry::{CacheEntry, CacheEntryInner},
    loader::Loader,
    utils::{HashMap, RandomState, BorrowedKey, Key, OwnedKey, Private, RwLock},
    source::{FileSystem, Source},
};

#[cfg(doc)]
use crate::{AssetGuard};

use std::{
    fmt,
    io,
    path::Path,
    sync::Arc,
};

#[cfg(feature = "hot-reloading")]
use crate::utils::HashSet;

#[cfg(feature = "hot-reloading")]
use std::{
    cell::Cell,
    ptr::NonNull,
};

type Shard = RwLock<HashMap<OwnedKey, CacheEntry>>;

pub(crate) struct Map {
    hash_builder: RandomState,
    shards: Box<[Shard]>,
}

impl Map {
    fn new(min_shards: usize) -> Map {
        let shards = min_shards.next_power_of_two();

        let hash_builder = RandomState::new();
        let shards = (0..shards).map(|_| {
            let map = HashMap::with_hasher(hash_builder.clone());
            RwLock::new(map)
        }).collect();

        Map { hash_builder, shards }
    }

    fn get_shard(&self, key: &dyn Key) -> &Shard {
        use std::hash::*;

        let mut hasher = self.hash_builder.build_hasher();
        key.hash(&mut hasher);
        let id = (hasher.finish() as usize) & (self.shards.len() - 1);
        &self.shards[id]
    }

    fn get_shard_mut(&mut self, key: &dyn Key) -> &mut Shard {
        use std::hash::*;

        let mut hasher = self.hash_builder.build_hasher();
        key.hash(&mut hasher);
        let id = (hasher.finish() as usize) & (self.shards.len() - 1);
        &mut self.shards[id]
    }

    pub fn get_entry(&self, key: &dyn Key) -> Option<CacheEntryInner> {
        let shard = self.get_shard(key).read();
        let entry = shard.get(key)?;
        unsafe { Some(entry.inner().extend_lifetime()) }
    }

    #[cfg(feature = "hot-reloading")]
    pub fn get_key_entry(&self, key: &dyn Key) -> Option<(OwnedKey, CacheEntryInner)> {
        let shard = self.get_shard(key).read();
        let (key, entry) = shard.get_key_value(key)?;
        unsafe { Some((key.into(), entry.inner().extend_lifetime())) }
    }

    fn insert(&self, key: OwnedKey, entry: CacheEntry) -> CacheEntryInner {
        let shard = &mut *self.get_shard(&key).write();
        let entry = shard.entry(key).or_insert(entry);
        unsafe { entry.inner().extend_lifetime() }
    }

    #[cfg(feature = "hot-reloading")]
    pub fn update_or_insert<T>(
        &self, key: OwnedKey, val: T,
        on_occupied: impl FnOnce(T, &CacheEntry),
        on_vacant: impl FnOnce(T, Arc<str>) -> CacheEntry,
    ) {
        use std::collections::hash_map::Entry;
        let shard = &mut *self.get_shard(&key).write();

        match shard.entry(key) {
            Entry::Occupied(entry) => on_occupied(val, entry.get()),
            Entry::Vacant(entry) => {
                let id = entry.key().clone().into_id();
                entry.insert(on_vacant(val, id));
            }
        }
    }

    fn contains_key(&self, key: &dyn Key) -> bool {
        let shard = self.get_shard(key).read();
        shard.contains_key(key)
    }

    fn take(&mut self, key: &dyn Key) -> Option<CacheEntry> {
        self.get_shard_mut(key).get_mut().remove(key)
    }

    fn remove(&mut self, key: &dyn Key) -> bool {
        self.get_shard_mut(key).get_mut().remove(key).is_some()
    }

    fn clear(&mut self) {
        for shard in &mut *self.shards {
            shard.get_mut().clear();
        }
    }
}

impl fmt::Debug for Map {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();

        for shard in &*self.shards {
            map.entries(&**shard.read());
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
    pub(crate) assets: Map,
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

impl<S> AssetCache<S>
where
    S: Source,
{
    /// Creates a cache that loads assets from the given source.
    pub fn with_source(source: S) -> AssetCache<S> {
        AssetCache {
            assets: Map::new(32),
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
        let id = Arc::<str>::from(id);
        let entry = CacheEntry::new(asset, id.clone(), A::HOT_RELOADED);
        let key = OwnedKey::new::<A>(id);

        let handle = self.assets.insert(key, entry).handle();
        Ok(handle)
    }

    /// Adds any value to the cache.
    #[cold]
    fn add_any<A: Send + Sync + 'static>(&self, id: &str, asset: A) -> Handle<A> {
        let id = Arc::<str>::from(id);
        let entry = CacheEntry::new(asset, id.clone(), false);
        let key = OwnedKey::new::<A>(id);

        self.assets.insert(key, entry).handle()
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
        match self.get_cached(id) {
            Some(asset) => Ok(asset),
            None => self.add_asset(id),
        }
    }

    /// Gets a value from the cache.
    ///
    /// The value does not have to be an asset, but if it is not, its type must
    /// be marked with the [`Storable`] trait.
    ///
    /// This function does not attempt to load the value from the source if it
    /// is not found in the cache.
    pub fn get_cached<A: Storable>(&self, id: &str) -> Option<Handle<A>> {
        let key = &BorrowedKey::new::<A>(id);

        #[cfg(not(feature = "hot-reloading"))]
        let entry = self.assets.get_entry(key)?;

        #[cfg(feature = "hot-reloading")]
        let entry = if A::HOT_RELOADED {
            match self.assets.get_key_entry(key) {
                Some((key, entry)) => {
                    self.add_record(key);
                    Some(entry)
                },
                None => {
                    let key = BorrowedKey::new::<A>(id);
                    self.add_record(key);
                    None
                },
            }
        } else {
            self.assets.get_entry(key)
        }?;

        Some(entry.handle())
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// As for `get_cached`, non-assets types must be marked with [`Storable`].
    #[inline]
    pub fn get_or_insert<A: Storable>(&self, id: &str, default: A) -> Handle<A> {
        match self.get_cached(id) {
            Some(handle) => handle,
            None => self.add_any(id, default),
        }
    }

    /// Returns `true` if the cache contains the specified asset.
    #[inline]
    pub fn contains<A: Compound>(&self, id: &str) -> bool {
        let key = &BorrowedKey::new::<A>(id);
        self.assets.contains_key(key)
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
    pub fn load_dir<A: DirLoadable>(&self, id: &str, recursive: bool) -> Result<DirHandle<A, S>, Error> {
        Ok(if recursive {
            let handle = self.load(id)?;
            DirHandle::new_rec(handle, self)
        } else {
            let handle = self.load(id)?;
            DirHandle::new(handle, self)
        })
    }

    /// Loads an directory from the cache.
    ///
    /// This function does not attempt to load the it from the source if it is
    /// not found in the cache.
    #[inline]
    pub fn get_cached_dir<A: DirLoadable>(&self, id: &str, recursive: bool) -> Option<DirHandle<A, S>> {
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
        #[cfg(feature = "hot-reloading")]
        if A::HOT_RELOADED && self.is_recording() {
            let key = BorrowedKey::new::<A>(id);
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
        let key = &BorrowedKey::new::<A>(id);
        self.assets.remove(key)
    }

    /// Takes ownership on a cached asset.
    ///
    /// The corresponding asset is removed from the cache.
    #[inline]
    pub fn take<A: Compound>(&mut self, id: &str) -> Option<A> {
        let key = &BorrowedKey::new::<A>(id);
        self.assets.take(key).map(|e| e.into_inner())
    }

    /// Clears the cache.
    ///
    /// Removes all cached assets and directories.
    #[inline]
    pub fn clear(&mut self) {
        self.assets.clear();

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
    /// free to keep [`Handle`]s, though.
    ///
    /// If `self.source()` was created without hot-reloading or if it failed to
    /// start, this function is a no-op.
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    #[inline]
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
    ///
    /// If `self.source()` was created without hot-reloading or if it failed to
    /// start, this function is a no-op.
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    #[inline]
    pub fn enhance_hot_reloading(&'static self) {
        if let Some(reloader) = &self.source.reloader {
            reloader.send_static(self);
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
