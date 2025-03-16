//! This module defines `AnyCache` and many traits to reduce code redondancy.
//!
//! There are 3 traits here: `RawCache`, `Cache`, `CacheExt`.
//! The goal of this is to have an object-safe cache trait to use in
//! `AnyCache`, while not losing the ability to use caches without virtual
//! calls.
//!
//! - The `Cache` is the central one, and is designed to be object safe.
//! - The `RawCache` is there to ease implementations of `Cache` without
//!   repeating code.
//! - The `CacheExt` adds generics on top of `Cache` to ease the use of
//!   `Cache`'s methods.

use std::{any::TypeId, fmt, io};

use crate::{
    Compound, Error, Handle, SharedString, Storable,
    asset::DirLoadable,
    entry::{CacheEntry, UntypedHandle},
    key::Type,
    source::{DirEntry, Source},
};

#[cfg(feature = "hot-reloading")]
use crate::hot_reloading::{Dependencies, HotReloader, records};

#[cfg(doc)]
use crate::AssetCache;

/// A non-generic version of [`AssetCache`].
///
/// For most purposes, this can be used exactly like an `AssetCache`: you can
/// load assets from it.
///
/// Unlike `AssetCache` this type is not generic, which is useful to make nicer
/// APIs.
#[derive(Clone, Copy)]
pub struct AnyCache<'a> {
    cache: &'a dyn Cache,
}

#[derive(Clone, Copy)]
struct AnySource<'a> {
    cache: &'a dyn Cache,
}

impl Source for AnySource<'_> {
    #[inline]
    fn read(&self, id: &str, ext: &str) -> io::Result<crate::source::FileContent> {
        self.cache.read(id, ext)
    }

    #[inline]
    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        self.cache.read_dir(id, f)
    }

    #[inline]
    fn exists(&self, entry: DirEntry) -> bool {
        self.cache.exists(entry)
    }
}

impl<'a> AnyCache<'a> {
    /// The `Source` from which assets are loaded.
    #[inline]
    pub fn raw_source(self) -> impl Source + 'a {
        AnySource { cache: self.cache }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub(crate) fn reloader(self) -> Option<&'a HotReloader> {
        self.cache.reloader()
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
    pub fn load<T: Compound>(self, id: &str) -> Result<&'a Handle<T>, Error> {
        self.cache._load(id)
    }

    /// Loads an asset and panic if an error happens.
    ///
    /// # Panics
    ///
    /// Panics if an error happens while loading the asset (see [`load`]).
    ///
    /// [`load`]: `Self::load`
    #[inline]
    pub fn load_expect<T: Compound>(self, id: &str) -> &'a Handle<T> {
        self.cache._load_expect(id)
    }

    /// Gets a value from the cache.
    ///
    /// This function does not attempt to load the value from the source if it
    /// is not found in the cache.
    #[inline]
    pub fn get_cached<T: Storable>(self, id: &str) -> Option<&'a Handle<T>> {
        self.cache._get_cached(id)
    }

    /// Gets a value with the given type from the cache.
    ///
    /// This is an equivalent of `get_cached` but with a dynamic type.
    #[inline]
    pub fn get_cached_untyped(self, id: &str, type_id: TypeId) -> Option<&'a UntypedHandle> {
        self.cache.get_cached_entry(id, type_id)
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// As for `get_cached`, non-assets types must be marked with [`Storable`].
    ///
    /// Assets added via this function will *never* be reloaded.
    #[inline]
    pub fn get_or_insert<T: Storable>(self, id: &str, default: T) -> &'a Handle<T> {
        self.cache._get_or_insert(id, default)
    }

    /// Returns `true` if the cache contains the specified asset.
    #[inline]
    pub fn contains<T: Storable>(self, id: &str) -> bool {
        self.cache._contains::<T>(id)
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
        self,
        id: &str,
    ) -> Result<&'a Handle<crate::Directory<T>>, Error> {
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
        self,
        id: &str,
    ) -> Result<&'a Handle<crate::RecursiveDirectory<T>>, Error> {
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
    pub fn load_owned<T: Compound>(self, id: &str) -> Result<T, Error> {
        self.cache._load_owned(id)
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
    pub fn no_record<T, F: FnOnce() -> T>(self, f: F) -> T {
        #[cfg(feature = "hot-reloading")]
        {
            records::no_record(f)
        }

        #[cfg(not(feature = "hot-reloading"))]
        {
            f()
        }
    }

    /// Returns `true` if values stored in this cache may be hot-reloaded.
    #[inline]
    pub fn is_hot_reloaded(self) -> bool {
        self.cache._has_reloader()
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn reload_untyped(self, id: SharedString, typ: Type) -> Option<Dependencies> {
        let handle = self.get_cached_untyped(&id, typ.type_id)?;

        let load_asset = || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (typ.inner.load)(self, id)))
        };
        let (entry, deps) = if let Some(reloader) = self.reloader() {
            records::record(reloader, load_asset)
        } else {
            log::warn!("No reloader in hot-reloading context");
            (load_asset(), Dependencies::new())
        };
        match entry {
            Ok(Ok(e)) => {
                handle.write(e);
                log::info!("Reloading \"{}\"", handle.id());
                Some(deps)
            }
            Ok(Err(err)) => {
                log::warn!("Error reloading \"{}\": {}", err.id(), err.reason());
                None
            }
            Err(_) => {
                log::warn!("Panic while reloading asset");
                None
            }
        }
    }
}

impl fmt::Debug for AnyCache<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyCache").finish_non_exhaustive()
    }
}

pub(crate) trait AssetMap {
    fn get(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle>;

    fn insert(&self, entry: CacheEntry) -> &UntypedHandle;

    fn contains_key(&self, id: &str, type_id: TypeId) -> bool;
}

pub(crate) trait Cache {
    #[cfg(feature = "hot-reloading")]
    fn reloader(&self) -> Option<&HotReloader>;

    fn read(&self, id: &str, ext: &str) -> io::Result<crate::source::FileContent>;

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()>;

    fn exists(&self, entry: DirEntry) -> bool;

    fn get_cached_entry(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle>;

    fn contains(&self, id: &str, type_id: TypeId) -> bool;

    fn load_entry(&self, id: &str, typ: Type) -> Result<&UntypedHandle, Error>;

    fn insert(&self, entry: CacheEntry) -> &UntypedHandle;
}

pub(crate) trait RawCache: Sized {
    type AssetMap: AssetMap;
    type Source: Source;

    fn assets(&self) -> &Self::AssetMap;

    fn get_source(&self) -> &Self::Source;

    #[cfg(feature = "hot-reloading")]
    fn reloader(&self) -> Option<&HotReloader>;

    #[cold]
    fn add_asset(&self, id: &str, typ: Type) -> Result<&UntypedHandle, Error> {
        log::trace!("Loading \"{}\"", id);

        let id = SharedString::from(id);
        let cache = AnyCache { cache: self };
        let entry = crate::asset::load_and_record(cache, id, typ)?;

        Ok(self.assets().insert(entry))
    }
}

impl<T: RawCache> Cache for T {
    #[cfg(feature = "hot-reloading")]
    #[inline]
    fn reloader(&self) -> Option<&HotReloader> {
        self.reloader()
    }

    fn read(&self, id: &str, ext: &str) -> io::Result<crate::source::FileContent> {
        #[cfg(feature = "hot-reloading")]
        if let Some(reloader) = self.reloader() {
            records::add_file_record(reloader, id, ext);
        }
        self.get_source().read(id, ext)
    }

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        #[cfg(feature = "hot-reloading")]
        if let Some(reloader) = self.reloader() {
            records::add_dir_record(reloader, id);
        }
        self.get_source().read_dir(id, f)
    }

    fn exists(&self, entry: DirEntry) -> bool {
        self.get_source().exists(entry)
    }

    fn get_cached_entry(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        #[cfg(feature = "hot-reloading")]
        if let Some(reloader) = self.reloader() {
            let (id, entry) = match self.assets().get(id, type_id) {
                Some(entry) => (entry.id().clone(), Some(entry)),
                None => (id.into(), None),
            };
            records::add_record(reloader, id, type_id);
            return entry;
        }

        self.assets().get(id, type_id)
    }

    #[inline]
    fn contains(&self, id: &str, type_id: TypeId) -> bool {
        self.assets().contains_key(id, type_id)
    }

    fn load_entry(&self, id: &str, typ: Type) -> Result<&UntypedHandle, Error> {
        match self.get_cached_entry(id, typ.type_id) {
            Some(entry) => Ok(entry),
            None => self.add_asset(id, typ),
        }
    }

    #[inline]
    fn insert(&self, entry: CacheEntry) -> &UntypedHandle {
        self.assets().insert(entry)
    }
}

pub(crate) trait CacheExt: Cache {
    fn _as_any_cache(&self) -> AnyCache;

    #[inline]
    fn _has_reloader(&self) -> bool {
        #[cfg(not(feature = "hot-reloading"))]
        return false;

        #[cfg(feature = "hot-reloading")]
        self.reloader().is_some()
    }

    #[inline]
    fn _get_cached<T: Storable>(&self, id: &str) -> Option<&Handle<T>> {
        let entry = self.get_cached_entry(id, TypeId::of::<T>())?;
        Some(entry.downcast_ref_ok())
    }

    #[cold]
    fn add_any<T: Storable>(&self, id: &str, asset: T) -> &UntypedHandle {
        let id = SharedString::from(id);
        let entry = CacheEntry::new_any(asset, id, false);

        self.insert(entry)
    }

    fn _get_or_insert<T: Storable>(&self, id: &str, default: T) -> &Handle<T> {
        let entry = match self.get_cached_entry(id, TypeId::of::<T>()) {
            Some(entry) => entry,
            None => self.add_any(id, default),
        };

        entry.downcast_ref_ok()
    }

    #[inline]
    fn _contains<T: Storable>(&self, id: &str) -> bool {
        self.contains(id, TypeId::of::<T>())
    }

    fn _load<T: Compound>(&self, id: &str) -> Result<&Handle<T>, Error> {
        let entry = self.load_entry(id, Type::of_asset::<T>())?;
        Ok(entry.downcast_ref_ok())
    }

    #[inline]
    #[track_caller]
    fn _load_expect<T: Compound>(&self, id: &str) -> &Handle<T> {
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
        match self._load(id) {
            Ok(h) => h,
            Err(err) => expect_failed(err),
        }
    }

    #[inline]
    fn _load_owned<T: Compound>(&self, id: &str) -> Result<T, Error> {
        let id = SharedString::from(id);
        T::load(self._as_any_cache(), &id).map_err(|err| Error::new(id, err))
    }
}

impl<T: Cache> CacheExt for T {
    #[inline]
    fn _as_any_cache(&self) -> AnyCache {
        AnyCache { cache: self }
    }
}

impl CacheExt for dyn Cache + '_ {
    #[inline]
    fn _as_any_cache(&self) -> AnyCache {
        AnyCache { cache: self }
    }
}

/// Used to get an `AnyCache` from a type.
///
/// This is useful to make generic functions that can work with any cache type.
pub trait AsAnyCache<'a> {
    /// Converts this type to an `AnyCache`.
    fn as_any_cache(&self) -> AnyCache<'a>;
}

impl<'a> AsAnyCache<'a> for AnyCache<'a> {
    #[inline]
    fn as_any_cache(&self) -> AnyCache<'a> {
        *self
    }
}

impl<'a, T: AsAnyCache<'a>> AsAnyCache<'a> for &'_ T {
    fn as_any_cache(&self) -> AnyCache<'a> {
        T::as_any_cache(self)
    }
}
