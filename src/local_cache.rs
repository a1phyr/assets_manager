use crate::{
    anycache::CacheExt,
    asset::DirLoadable,
    entry::{CacheEntry, UntypedHandle},
    source::Source,
    utils::RandomState,
    AnyCache, Compound, Error, Handle, Storable,
};
use std::{any::TypeId, cell::RefCell, fmt};

#[cfg(doc)]
use crate::AssetReadGuard;

pub(crate) struct AssetMap {
    map: RefCell<crate::map::AssetMap>,
    hash_builder: RandomState,
}

impl AssetMap {
    fn new() -> AssetMap {
        AssetMap {
            map: RefCell::new(crate::map::AssetMap::new()),
            hash_builder: RandomState::new(),
        }
    }

    fn hash_one(&self, key: (TypeId, &str)) -> u64 {
        std::hash::BuildHasher::hash_one(&self.hash_builder, key)
    }

    fn take(&mut self, id: &str, type_id: TypeId) -> Option<CacheEntry> {
        let hash = self.hash_one((type_id, id));
        self.map.get_mut().take(hash, id, type_id)
    }

    fn remove(&mut self, id: &str, type_id: TypeId) -> bool {
        self.take(id, type_id).is_some()
    }

    fn clear(&mut self) {
        self.map.get_mut().clear();
    }
}

impl crate::anycache::AssetMap for AssetMap {
    fn get(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        let hash = self.hash_one((type_id, id));
        unsafe { Some(self.map.borrow().get(hash, id, type_id)?.extend_lifetime()) }
    }

    fn insert(&self, entry: CacheEntry) -> &UntypedHandle {
        let hash = self.hash_one(entry.as_key());
        unsafe {
            self.map
                .borrow_mut()
                .insert(hash, entry, &self.hash_builder)
                .extend_lifetime()
        }
    }

    fn contains_key(&self, id: &str, type_id: TypeId) -> bool {
        let hash = self.hash_one((type_id, id));
        self.map.borrow().get(hash, id, type_id).is_some()
    }
}

impl fmt::Debug for AssetMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.map.borrow().iter_for_debug())
            .finish()
    }
}

/// Single-threaded version of `AssetCache`.
///
/// This type is not thread-safe, but is cheaper if you don't need
/// synchronization. It still requires stored assets to be thread-safe.
///
/// This cache **does not** support hot-reloading.
pub struct LocalAssetCache<S = crate::source::FileSystem> {
    source: S,
    assets: AssetMap,
}

impl<S: Source> crate::anycache::RawCache for LocalAssetCache<S> {
    type AssetMap = AssetMap;
    type Source = S;

    #[inline]
    fn assets(&self) -> &AssetMap {
        &self.assets
    }

    #[inline]
    fn get_source(&self) -> &S {
        &self.source
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    fn reloader(&self) -> Option<&crate::hot_reloading::HotReloader> {
        None
    }
}

impl LocalAssetCache {
    /// Creates a new `LocalAssetCache` that reads assets from the given directory.
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let source = crate::source::FileSystem::new(path)?;
        Ok(Self::with_source(source))
    }
}

impl<S> LocalAssetCache<S> {
    /// Creates a new `LocalAssetCache` with the given source.
    #[inline]
    pub fn with_source(source: S) -> Self {
        Self {
            source,
            assets: AssetMap::new(),
        }
    }
}

impl<S: Source> LocalAssetCache<S> {
    /// Gets a value from the cache.
    ///
    /// See [`AnyCache::get_cached`] for more details.
    #[inline]
    pub fn get_cached<T: Storable>(&self, id: &str) -> Option<&Handle<T>> {
        self._get_cached(id)
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// See [`AnyCache::get_or_insert`] for more details.
    #[inline]
    pub fn get_or_insert<T: Storable>(&self, id: &str, default: T) -> &Handle<T> {
        self._get_or_insert(id, default)
    }

    /// Returns `true` if the cache contains the specified asset.
    ///
    /// See [`AnyCache::contains`] for more details.
    #[inline]
    pub fn contains<T: Storable>(&self, id: &str) -> bool {
        self._contains::<T>(id)
    }

    /// Loads an asset.
    ///
    /// See [`AnyCache::load`] for more details.
    #[inline]
    pub fn load<T: Compound>(&self, id: &str) -> Result<&Handle<T>, Error> {
        self._load(id)
    }

    /// Loads an asset and panic if an error happens.
    ///
    /// See [`AnyCache::load_expect`] for more details.
    #[inline]
    pub fn load_expect<T: Compound>(&self, id: &str) -> &Handle<T> {
        self._load_expect(id)
    }

    /// Loads all assets of a given type from a directory.
    ///
    /// See [`AnyCache::load_dir`] for more details.
    #[inline]
    pub fn load_dir<T: DirLoadable>(
        &self,
        id: &str,
    ) -> Result<&Handle<crate::Directory<T>>, Error> {
        self.load::<crate::Directory<T>>(id)
    }

    /// Loads all assets of a given type from a directory.
    ///
    /// See [`AnyCache::load_dir`] for more details.
    #[inline]
    pub fn load_rec_dir<T: DirLoadable>(
        &self,
        id: &str,
    ) -> Result<&Handle<crate::RecursiveDirectory<T>>, Error> {
        self.load::<crate::RecursiveDirectory<T>>(id)
    }

    /// Loads an owned version of an asset.
    ///
    /// See [`AnyCache::load_owned`] for more details.
    #[inline]
    pub fn load_owned<T: Compound>(&self, id: &str) -> Result<T, Error> {
        self._load_owned(id)
    }

    /// Converts to an `AnyCache`.
    #[inline]
    pub fn as_any_cache(&self) -> AnyCache {
        self._as_any_cache()
    }
}

impl<S> LocalAssetCache<S> {
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
        let (asset, _id) = self.assets.take(id, TypeId::of::<T>())?.into_inner();
        Some(asset)
    }

    /// Clears the cache.
    ///
    /// Removes all cached assets and directories.
    #[inline]
    pub fn clear(&mut self) {
        self.assets.clear();
    }
}

impl<S> Default for LocalAssetCache<S>
where
    S: Source + Default,
{
    #[inline]
    fn default() -> Self {
        Self::with_source(S::default())
    }
}

impl<'a, S: Source> crate::AsAnyCache<'a> for &'a LocalAssetCache<S> {
    #[inline]
    fn as_any_cache(&self) -> AnyCache<'a> {
        (*self).as_any_cache()
    }
}

impl<S> fmt::Debug for LocalAssetCache<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalAssetCache")
            .field("assets", &self.assets)
            .finish()
    }
}
