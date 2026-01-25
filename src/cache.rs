//! Definition of the cache

use crate::{
    Asset, Error, Handle, SharedString,
    asset::{DirLoadable, Storable},
    entry::{CacheEntry, UntypedHandle},
    key::Type,
    map::{AssetMap, Hasher},
    source::{FileSystem, Source},
};

#[cfg(doc)]
use crate::AssetReadGuard;

use std::{any::TypeId, fmt, io, path::Path, sync::Arc};

#[cfg(feature = "hot-reloading")]
use crate::hot_reloading::{AssetKey, HotReloader, records};

#[cfg(feature = "hot-reloading")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CacheId(usize);

/// The main structure of this crate, used to load and store assets.
///
/// It uses interior mutability, so assets can be added in the cache without
/// requiring a mutable reference, but one is required to remove an asset.
///
/// Within the cache, assets are identified with their type and a string. This
/// string is constructed from the asset path, replacing `/` by `.` and removing
/// the extension. Given that, you cannot use `.` in your file names except for
/// the extension.
///
/// # Example
///
/// ```
/// # cfg_if::cfg_if! { if #[cfg(all(feature = "ron", feature = "macros"))] {
/// use assets_manager::{Asset, AssetCache};
/// use serde::Deserialize;
///
/// #[derive(Debug, Deserialize, Asset)]
/// #[asset_format = "ron"]
/// struct Point {
///     x: i32,
///     y: i32,
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
/// #   break;
/// }
///
/// # }}
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct AssetCache(Arc<AssetCacheInner>);

enum CacheKind {
    Root {
        source: Box<dyn Source + Send + Sync>,
    },
    Node {
        parent: AssetCache,
    },
}

struct AssetCacheInner {
    #[cfg(feature = "hot-reloading")]
    reloader: Option<HotReloader>,

    assets: AssetMap,
    hasher: Hasher,
    kind: CacheKind,
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
        Self::_with_source(Box::new(source))
    }

    fn _with_source(source: Box<dyn Source + Send + Sync>) -> Self {
        Self(Arc::new(AssetCacheInner {
            #[cfg(feature = "hot-reloading")]
            reloader: HotReloader::start(&*source),

            assets: AssetMap::new(),
            hasher: Hasher::default(),
            kind: CacheKind::Root { source },
        }))
    }

    /// Creates a cache that loads assets from the given source.
    pub fn without_hot_reloading<S: Source + Send + Sync + 'static>(source: S) -> Self {
        Self(Arc::new(AssetCacheInner {
            #[cfg(feature = "hot-reloading")]
            reloader: None,

            assets: AssetMap::new(),
            hasher: Hasher::default(),
            kind: CacheKind::Root {
                source: Box::new(source),
            },
        }))
    }

    /// Makes a new cache with `self` as parent.
    pub fn make_child(&self) -> Self {
        Self(Arc::new(AssetCacheInner {
            #[cfg(feature = "hot-reloading")]
            reloader: self.0.reloader.clone(),

            assets: AssetMap::new(),
            hasher: self.0.hasher.clone(),
            kind: CacheKind::Node {
                parent: self.clone(),
            },
        }))
    }

    /// Returns a reference to the cache's [`Source`].
    #[inline]
    pub fn source(&self) -> impl Source + Send + Sync + '_ {
        CacheSource {
            source: self.get_raw_source(),
            #[cfg(feature = "hot-reloading")]
            is_hot_reloaded: self.is_hot_reloaded(),
        }
    }

    /// Returns a reference to the cache's [`Source`].
    #[inline]
    pub fn downcast_raw_source<S: Source + 'static>(&self) -> Option<&S> {
        self.get_raw_source().downcast_ref()
    }

    fn get_raw_source(&self) -> &(dyn Source + Send + Sync + 'static) {
        let mut cur = self;
        loop {
            match &cur.0.kind {
                CacheKind::Node { parent } => cur = parent,
                CacheKind::Root { source } => return &**source,
            }
        }
    }

    /// Returns a reference to the cache's parent, if any.
    #[inline]
    pub fn parent(&self) -> Option<&Self> {
        match &self.0.kind {
            CacheKind::Root { .. } => None,
            CacheKind::Node { parent } => Some(parent),
        }
    }

    /// Returns an iterator over `self` and its ancestors.
    ///
    /// ```
    /// let cache = assets_manager::AssetCache::new("assets")?;
    /// let child = cache.make_child();
    /// let grandchild = child.make_child();
    ///
    /// let mut ancestors = grandchild.ancestors();
    ///
    /// assert!(ancestors.next().is_some()); // `grandchild`
    /// assert!(ancestors.next().is_some()); // `child`
    /// assert!(ancestors.next().is_some()); // `cache`
    /// assert!(ancestors.next().is_none());
    ///
    /// # Ok::<_, assets_manager::BoxedError>(())
    /// ```
    #[inline]
    pub fn ancestors(&self) -> impl Iterator<Item = &AssetCache> {
        let mut next = Some(self);

        std::iter::from_fn(move || {
            let cur = next?;
            next = cur.parent();
            Some(cur)
        })
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn downgrade(&self) -> WeakAssetCache {
        WeakAssetCache(Arc::downgrade(&self.0))
    }

    /// Temporarily prevent `Asset` dependencies to be recorded.
    ///
    /// This function disables dependencies recording in [`Asset::load`].
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
        if self.0.reloader.is_some() && handle.typ().is_some() {
            let key = || AssetKey::new(handle.id().clone(), handle.type_id(), self.downgrade());
            records::add_asset(key);
        }
    }

    /// Loads an asset.
    ///
    /// If the asset is not found in the cache, it is loaded from the source.
    #[inline]
    pub fn load<T: Asset>(&self, id: &str) -> Result<&Handle<T>, Error> {
        let handle = self.load_untyped(id, Type::of_asset::<T>())?;
        Ok(handle.downcast_ref_ok())
    }

    /// Loads an asset and panic if an error happens.
    ///
    /// # Panics
    ///
    /// Panics if an error happens while loading the asset.
    #[inline]
    #[track_caller]
    pub fn load_expect<T: Asset>(&self, id: &str) -> &Handle<T> {
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

    fn load_untyped(&self, id: &str, typ: &Type) -> Result<&UntypedHandle, Error> {
        match self.get_untyped(id, typ.type_id) {
            Some(handle) => Ok(handle),
            None => self.add_asset(id, typ),
        }
    }

    #[cold]
    fn add_asset(&self, id: &str, typ: &Type) -> Result<&UntypedHandle, Error> {
        log::trace!("Loading \"{id}\"");

        let id = SharedString::from(id);

        if crate::utils::is_invalid_id(&id) {
            return Err(Error::new(id, crate::error::ErrorKind::InvalidId.into()));
        }

        #[cfg(feature = "hot-reloading")]
        if typ.hot_reloaded
            && let Some(reloader) = &self.0.reloader
        {
            let (result, deps) = crate::hot_reloading::records::record(|| (typ.load)(self, id));
            let entry = result?;

            let key = AssetKey::new(entry.inner().id().clone(), typ.type_id, self.downgrade());
            reloader.add_asset(key, deps);

            let handle = self.0.assets.insert(entry, &self.0.hasher);
            self.add_record(handle);
            return Ok(handle);
        }

        let entry = (typ.load)(self, id)?;
        Ok(self.0.assets.insert(entry, &self.0.hasher))
    }

    /// Gets a value from the cache.
    ///
    /// This function does not attempt to load the value from the source if it
    /// is not found in the cache.
    #[inline]
    pub fn get<T: Storable>(&self, id: &str) -> Option<&Handle<T>> {
        let handle = self.get_untyped(id, TypeId::of::<T>())?;
        Some(handle.downcast_ref_ok())
    }

    /// Deprecated name of `get`.
    #[inline]
    #[deprecated = "Use `get` instead"]
    pub fn get_cached<T: Storable>(&self, id: &str) -> Option<&Handle<T>> {
        self.get(id)
    }

    /// Gets a value with the given type from the cache.
    ///
    /// This is an equivalent of `get` but with a dynamic type.
    pub fn get_untyped(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        let key = &(type_id, id);
        let hash = self.0.hasher.hash_key(key);

        let mut cur = self;

        loop {
            if let Some(handle) = cur.0.assets.get(hash, key) {
                #[cfg(feature = "hot-reloading")]
                cur.add_record(handle);

                return Some(handle);
            }
            cur = cur.parent()?;
        }
    }

    /// Deprecated name of `get_untyped`.
    #[deprecated = "Use `get` instead"]
    pub fn get_cached_untyped(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        self.get_untyped(id, type_id)
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// Assets added via this function will *never* be reloaded.
    #[inline]
    pub fn get_or_insert<T: Storable>(&self, id: &str, default: T) -> &Handle<T> {
        let handle = match self.get_untyped(id, TypeId::of::<T>()) {
            Some(handle) => handle,
            None => self.add_any(id, default),
        };

        handle.downcast_ref_ok()
    }

    #[cold]
    fn add_any<T: Storable>(&self, id: &str, asset: T) -> &UntypedHandle {
        let id = SharedString::from(id);
        let entry = CacheEntry::new_any(asset, id, false);

        self.0.assets.insert(entry, &self.0.hasher)
    }

    /// Returns `true` if the cache contains the specified asset.
    #[inline]
    pub fn contains<T: Storable>(&self, id: &str) -> bool {
        self.contains_untyped(id, TypeId::of::<T>())
    }

    /// Returns `true` if the cache contains the specified asset.
    fn contains_untyped(&self, id: &str, type_id: TypeId) -> bool {
        let key = &(type_id, id);
        let hash = self.0.hasher.hash_key(key);

        let mut cur = self;

        loop {
            if cur.0.assets.get(hash, key).is_some() {
                return true;
            }

            match &cur.0.kind {
                CacheKind::Root { .. } => return false,
                CacheKind::Node { parent } => cur = parent,
            }
        }
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
    pub fn load_dir<T: DirLoadable + Asset>(
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
    pub fn load_rec_dir<T: DirLoadable + Asset>(
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
    /// Inside an implementation [`Asset::load`], you should use `T::load`
    /// directly.
    #[inline]
    pub fn load_owned<T: Asset>(&self, id: &str) -> Result<T, Error> {
        let id = SharedString::from(id);
        T::load(self, &id).map_err(|err| Error::new(id, err))
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn reload_untyped(&self, key: &AssetKey) -> Option<records::Dependencies> {
        let handle = self.get_untyped(&key.id, key.type_id)?;

        let id = handle.id();
        let typ = handle.typ()?;

        let (entry, deps) = records::record(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (typ.load)(self, id.clone())
            }))
        });

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
        return self.0.reloader.is_some();

        #[cfg(not(feature = "hot-reloading"))]
        false
    }

    /// Iterate on all assets in this cache and its ancestors.
    ///
    /// # Deadlocks
    ///
    /// Using this function will cause deadlock if trying to insert asset in
    /// this cache or its ancestors in the current thread while the iterator is
    /// alive. You can work around that by collecting the iterator to a `Vec`
    /// first.
    pub fn iter(&self) -> impl Iterator<Item = &UntypedHandle> {
        let all_shards: Vec<_> = self
            .ancestors()
            .flat_map(|c| c.0.assets.iter_shards())
            .collect();

        // We are building a self-referential struct here so we extend the
        // lifetime of the borrowed slice so the borrow-checker don't annoy us.
        #[allow(clippy::deref_addrof)]
        let slice = unsafe { &*(&raw const *all_shards) };

        slice.iter().flat_map(move |s| {
            // HACK: we need this variable to be moved in the iterator because
            // it holds the locks guards.
            let _shards = &all_shards;
            s.iter()
        })
    }

    /// Iterate on all assets of a given type in this cache and its ancestors.
    ///
    /// # Deadlocks
    ///
    /// Using this function will cause deadlock if trying to insert asset in
    /// this cache or its ancestors in the current thread while the iterator is
    /// alive. You can work around that by collecting the iterator to a `Vec`
    /// first.
    pub fn iter_by_type<T: Storable>(&self) -> impl Iterator<Item = &Handle<T>> {
        self.iter().filter_map(|h| h.downcast_ref())
    }
}

#[cfg(feature = "hot-reloading")]
impl Drop for AssetCacheInner {
    fn drop(&mut self) {
        if let Some(reloader) = &self.reloader {
            reloader.remove_cache(CacheId((self as *const Self).addr()));
        }
    }
}

impl fmt::Debug for AssetCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("assets", &self.0.assets)
            .finish()
    }
}

struct CacheSource<'a> {
    #[cfg(feature = "hot-reloading")]
    is_hot_reloaded: bool,
    source: &'a (dyn Source + Send + Sync),
}

impl Source for CacheSource<'_> {
    fn read(&self, id: &str, ext: &str) -> io::Result<crate::source::FileContent<'_>> {
        #[cfg(feature = "hot-reloading")]
        if self.is_hot_reloaded {
            records::add_file_record(id, ext);
        }
        self.source.read(id, ext)
    }

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(crate::source::DirEntry)) -> io::Result<()> {
        #[cfg(feature = "hot-reloading")]
        if self.is_hot_reloaded {
            records::add_dir_record(id);
        }
        self.source.read_dir(id, f)
    }

    fn exists(&self, entry: crate::source::DirEntry) -> bool {
        self.source.exists(entry)
    }
}

#[cfg(feature = "hot-reloading")]
#[derive(Debug, Clone)]
pub(crate) struct WeakAssetCache(std::sync::Weak<AssetCacheInner>);

#[cfg(feature = "hot-reloading")]
impl WeakAssetCache {
    pub fn upgrade(&self) -> Option<AssetCache> {
        self.0.upgrade().map(AssetCache)
    }

    pub fn id(&self) -> CacheId {
        CacheId(self.0.as_ptr().addr())
    }
}

#[cfg(feature = "hot-reloading")]
impl PartialEq for WeakAssetCache {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

#[cfg(feature = "hot-reloading")]
impl Eq for WeakAssetCache {}

#[cfg(feature = "hot-reloading")]
impl std::hash::Hash for WeakAssetCache {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}
