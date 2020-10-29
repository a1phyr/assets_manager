//! Definition of the cache
use crate::{
    Asset, Error,
    dirs::{CachedDir, DirReader},
    entry::{CacheEntry, AssetRef},
    loader::Loader,
    utils::{HashMap, RwLock},
    source::{FileSystem, Source},
};


use std::{
    any::TypeId,
    borrow::Borrow,
    fmt,
    io,
    path::Path,
};


/// The key used to identify assets
///
/// **Note**: This definition has to kept in sync with [`Key`]'s one.
///
/// [`Key`]: struct.Key.html
#[derive(PartialEq, Eq, Hash)]
#[repr(C)]
pub(crate) struct OwnedKey {
    id: Box<str>,
    type_id: TypeId,
}

impl OwnedKey {
    /// Creates a `OwnedKey` with the given type and id.
    #[inline]
    fn new<T: Asset>(id: Box<str>) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
        }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// A borrowed version of [`OwnedKey`]
///
/// [`OwnedKey`]: struct.OwnedKey.html
#[derive(PartialEq, Eq, Hash)]
#[repr(C)]
pub(crate) struct Key<'a> {
    id: &'a str,
    type_id: TypeId,
}

impl<'a> Key<'a> {
    /// Creates an Key for the given type and id.
    #[inline]
    pub fn new<T: Asset>(id: &'a str) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
        }
    }

    #[inline]
    #[cfg(feature = "hot-reloading")]
    pub fn new_with(id: &'a str, type_id: TypeId) -> Self {
        Self { id, type_id }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn id(&self) -> &str {
        self.id
    }
}

impl<'a> Borrow<Key<'a>> for OwnedKey {
    #[inline]
    fn borrow(&self) -> &Key<'a> {
        unsafe {
            let ptr = self as *const OwnedKey as *const Key;
            &*ptr
        }
    }
}

impl<'a> ToOwned for Key<'a> {
    type Owned = OwnedKey;

    #[inline]
    fn to_owned(&self) -> OwnedKey {
        OwnedKey {
            id: self.id.into(),
            type_id: self.type_id,
        }
    }
}

impl fmt::Debug for OwnedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key: &Key = self.borrow();
        key.fmt(f)
    }
}

impl fmt::Debug for Key<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedKey")
            .field("id", &self.id)
            .field("type_id", &self.type_id)
            .finish()
    }
}

/// The main structure of this crate, used to cache assets.
///
/// It uses interior mutability, so assets can be added in the cache without
/// requiring a mutable reference, but one is required to remove an asset.
///
/// Within the cache, assets are identified with their type and a string. This
/// string is constructed from the asset path, remplacing `/` by `.` and removing
/// the extension. Given that, you cannot use `.` in your file names except for
/// the extension.
///
/// **Note**: Using symbolic or hard links within the cached directory can lead
/// to surprising behaviour (especially with hot-reloading), and thus should be
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
/// let point_lock = cache.load::<Point>("common.position")?;
///
/// // Read it
/// let point = point_lock.read();
/// println!("Loaded position: {:?}", point);
/// # assert_eq!(point.x, 5);
/// # assert_eq!(point.y, -6);
///
/// // Release the lock
/// drop(point);
///
/// // Use hot-reloading
/// loop {
///     println!("Position: {:?}", point_lock.read());
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

    /// Returns a reference to the cache's [`Source`](source/trait.Source.html)
    #[inline]
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Adds an asset to the cache
    pub(crate) fn add_asset<A: Asset>(&self, id: Box<str>) -> Result<AssetRef<A>, Error> {
        #[cfg(feature = "hot-reloading")]
        self.source.__private_hr_add_asset::<A>(&id);

        let asset: A = load_from_source(&self.source, &id)?;

        let key = OwnedKey::new::<A>(id.clone());
        let mut assets = self.assets.write();

        let entry = assets.entry(key).or_insert_with(|| CacheEntry::new(asset, id));

        unsafe { Ok(entry.get_ref()) }
    }

    /// Adds a directory to the cache
    fn add_dir<A: Asset>(&self, id: Box<str>) -> Result<DirReader<A, S>, io::Error> {
        #[cfg(feature = "hot-reloading")]
        self.source.__private_hr_add_dir::<A>(&id);

        let dir = CachedDir::load::<A, S>(self, &id)?;

        let key = OwnedKey::new::<A>(id);
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
    /// - Loaded data could not not be converted properly
    /// - The asset has no extension
    pub fn load<A: Asset>(&self, id: &str) -> Result<AssetRef<A>, Error> {
        match self.load_cached(id) {
            Some(asset) => Ok(asset),
            None => self.add_asset(id.into()),
        }
    }

    /// Loads an asset from the cache.
    ///
    /// This function does not attempt to load the asset from the source if it
    /// is not found in the cache.
    pub fn load_cached<A: Asset>(&self, id: &str) -> Option<AssetRef<A>> {
        let key = Key::new::<A>(id);
        let cache = self.assets.read();
        cache.get(&key).map(|asset| unsafe { asset.get_ref() })
    }

    /// Loads an asset and panic if an error happens.
    ///
    /// # Panics
    ///
    /// Panics if an error happens while loading the asset (see [`load`]).
    ///
    /// [`load`]: fn.load.html
    #[inline]
    #[track_caller]
    pub fn load_expect<A: Asset>(&self, id: &str) -> AssetRef<A> {
        self.load(id).unwrap_or_else(|err| {
            panic!("Failed to load essential asset {:?}: {}", id, err)
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
    pub fn load_dir<A: Asset>(&self, id: &str) -> io::Result<DirReader<A, S>> {
        let dirs = self.dirs.read();
        match dirs.get(&Key::new::<A>(id)) {
            Some(dir) => unsafe { Ok(dir.read(self)) },
            None => {
                drop(dirs);
                self.add_dir(id.into())
            }
        }
    }

    /// Loads an owned version of an asset
    ///
    /// Note that the asset will not be fetched from the cache nor will it be
    /// cached. In addition, hot-reloading does not affect the returned value.
    ///
    /// This can be useful if you need ownership on a non-clonable value.
    #[inline]
    pub fn load_owned<A: Asset>(&self, id: &str) -> Result<A, Error> {
        load_from_source(&self.source, id)
    }

    /// Removes an asset from the cache, and returns whether it was present in
    /// the cache.
    ///
    /// Note that you need a mutable reference to the cache, so you cannot have
    /// any [`AssetRef`], [`AssetGuard`], etc when you call this function.
    ///
    /// [`AssetRef`]: struct.AssetRef.html
    /// [`AssetGuard`]: struct.AssetGuard.html
    #[inline]
    pub fn remove<A: Asset>(&mut self, id: &str) -> bool {
        let key = Key::new::<A>(id);
        let cache = self.assets.get_mut();
        cache.remove(&key).is_some()
    }

    /// Takes ownership on a cached asset.
    ///
    /// The corresponding asset is removed from the cache.
    pub fn take<A: Asset>(&mut self, id: &str) -> Option<A> {
        let key = Key::new::<A>(id);
        let cache = self.assets.get_mut();
        cache.remove(&key).map(|entry| unsafe { entry.into_inner() })
    }

    /// Clears the cache.
    ///
    /// Removes all cached assets and directories.
    #[inline]
    pub fn clear(&mut self) {
        self.assets.get_mut().clear();
        self.dirs.get_mut().clear();

        #[cfg(feature = "hot-reloading")]
        self.source.__private_hr_clear();
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
    /// free to keep [`AssetRef`]s, though. The same restriction applies to
    /// [`ReadDir`] and [`ReadAllDir`].
    ///
    /// [`AssetGuard`]: struct.AssetGuard.html
    /// [`AssetRef`]: struct.AssetRef.html
    /// [`ReadDir`]: struct.ReadDir.html
    /// [`ReadAllDir`]: struct.ReadAllDir.html
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    pub fn hot_reload(&self) {
        self.source.reloader.reload(self);
    }

    /// Enhances hot-reloading.
    ///
    /// Having a `'static` reference to the cache enables some optimisations,
    /// which you can take advantage of with this function. If an `AssetCache`
    /// is behind a `'static` reference, you should always prefer using this
    /// function over [`hot_reload`](#method.hot_reload).
    ///
    /// You only have to call this function once for it to take effect. After
    /// calling this function, subsequent calls to `hot_reload` and to this
    /// function have no effect.
    #[cfg(feature = "hot-reloading")]
    #[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
    pub fn enhance_hot_reloading(&'static self) {
        self.source.reloader.send_static(self);
    }
}

impl<S> fmt::Debug for AssetCache<S>
where
    S: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetCache")
            .field("source", &self.source)
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

fn load_from_source<A: Asset, S: Source>(source: &S, id: &str) -> Result<A, Error> {
    let mut error = Error::NoDefaultValue;

    for ext in A::EXTENSIONS {
        match load_single(source, id, ext) {
            Err(err) => error = err.or(error),
            asset => return asset,
        }
    }

    A::default_value(id, error)
}
