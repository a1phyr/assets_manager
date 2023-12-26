use crate::{
    anycache::CacheExt,
    asset::DirLoadable,
    entry::{CacheEntry, UntypedHandle},
    source::Source,
    utils::{BorrowedKey, HashMap, Key, OwnedKey},
    AnyCache, Compound, Error, Handle, SharedString, Storable,
};
use std::{any::TypeId, cell::RefCell, fmt};

#[cfg(doc)]
use crate::AssetReadGuard;

pub(crate) struct AssetMap {
    map: RefCell<HashMap<OwnedKey, CacheEntry>>,
}

impl AssetMap {
    fn new() -> AssetMap {
        AssetMap {
            map: RefCell::new(HashMap::new()),
        }
    }

    fn take(&mut self, id: &str, type_id: TypeId) -> Option<CacheEntry> {
        let key = BorrowedKey::new_with(id, type_id);
        self.map.get_mut().remove(&key as &dyn Key)
    }

    #[inline]
    fn remove(&mut self, id: &str, type_id: TypeId) -> bool {
        self.take(id, type_id).is_some()
    }

    fn clear(&mut self) {
        self.map.get_mut().clear();
    }
}

impl crate::anycache::AssetMap for AssetMap {
    fn get(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        let key = BorrowedKey::new_with(id, type_id);
        let map = self.map.borrow();
        let entry = map.get(&key as &dyn Key)?;
        unsafe { Some(entry.inner().extend_lifetime()) }
    }

    fn get_entry(&self, id: &str, type_id: TypeId) -> Option<(SharedString, &UntypedHandle)> {
        let key = BorrowedKey::new_with(id, type_id);
        let map = self.map.borrow();
        let (key, entry) = map.get_key_value(&key as &dyn Key)?;
        unsafe { Some((key.id.clone(), entry.inner().extend_lifetime())) }
    }

    fn insert(&self, id: SharedString, type_id: TypeId, entry: CacheEntry) -> &UntypedHandle {
        let key = OwnedKey::new_with(id, type_id);
        let mut map = self.map.borrow_mut();
        let entry = map.entry(key).or_insert(entry);
        unsafe { entry.inner().extend_lifetime() }
    }

    fn contains_key(&self, id: &str, type_id: TypeId) -> bool {
        let key = BorrowedKey::new_with(id, type_id);
        let map = self.map.borrow();
        map.contains_key(&key as &dyn Key)
    }
}

impl fmt::Debug for AssetMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.map.borrow().fmt(f)
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
    pub fn get_cached<A: Storable>(&self, id: &str) -> Option<&Handle<A>> {
        self._get_cached(id)
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// See [`AnyCache::get_or_insert`] for more details.
    #[inline]
    pub fn get_or_insert<A: Storable>(&self, id: &str, default: A) -> &Handle<A> {
        self._get_or_insert(id, default)
    }

    /// Returns `true` if the cache contains the specified asset.
    ///
    /// See [`AnyCache::contains`] for more details.
    #[inline]
    pub fn contains<A: Storable>(&self, id: &str) -> bool {
        self._contains::<A>(id)
    }

    /// Loads an asset.
    ///
    /// See [`AnyCache::load`] for more details.
    #[inline]
    pub fn load<A: Compound>(&self, id: &str) -> Result<&Handle<A>, Error> {
        self._load(id)
    }

    /// Loads an asset and panic if an error happens.
    ///
    /// See [`AnyCache::load_expect`] for more details.
    #[inline]
    pub fn load_expect<A: Compound>(&self, id: &str) -> &Handle<A> {
        self._load_expect(id)
    }

    /// Loads all assets of a given type from a directory.
    ///
    /// See [`AnyCache::load_dir`] for more details.
    #[inline]
    pub fn load_dir<A: DirLoadable>(
        &self,
        id: &str,
    ) -> Result<&Handle<crate::Directory<A>>, Error> {
        self.load::<crate::Directory<A>>(id)
    }

    /// Loads all assets of a given type from a directory.
    ///
    /// See [`AnyCache::load_dir`] for more details.
    #[inline]
    pub fn load_rec_dir<A: DirLoadable>(
        &self,
        id: &str,
    ) -> Result<&Handle<crate::RecursiveDirectory<A>>, Error> {
        self.load::<crate::RecursiveDirectory<A>>(id)
    }

    /// Loads an owned version of an asset.
    ///
    /// See [`AnyCache::load_owned`] for more details.
    #[inline]
    pub fn load_owned<A: Compound>(&self, id: &str) -> Result<A, Error> {
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
    pub fn remove<A: Storable>(&mut self, id: &str) -> bool {
        self.assets.remove(id, TypeId::of::<A>())
    }

    /// Takes ownership on a cached asset.
    ///
    /// The corresponding asset is removed from the cache.
    #[inline]
    pub fn take<A: Storable>(&mut self, id: &str) -> Option<A> {
        let (asset, _id) = self.assets.take(id, TypeId::of::<A>())?.into_inner();
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

impl<S: Source> crate::AsAnyCache for LocalAssetCache<S> {
    #[inline]
    fn as_any_cache(&self) -> AnyCache<'_> {
        self.as_any_cache()
    }
}

impl<S> fmt::Debug for LocalAssetCache<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalAssetCache")
            .field("assets", &self.assets)
            .finish()
    }
}
