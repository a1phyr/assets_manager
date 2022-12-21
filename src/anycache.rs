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

use std::{any::TypeId, borrow::Cow, fmt, io};

use crate::{
    asset::DirLoadable,
    cache::AssetMap,
    entry::{CacheEntry, UntypedHandle},
    key::Type,
    source::{DirEntry, Source},
    Compound, DirHandle, Error, Handle, SharedString, Storable,
};

#[cfg(feature = "hot-reloading")]
use crate::hot_reloading::{records, Dependencies, HotReloader};

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
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>> {
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
    pub fn source(self) -> impl Source + 'a {
        AnySource { cache: self.cache }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub(crate) fn reloader(&self) -> Option<&'a HotReloader> {
        self.cache.reloader()
    }

    /// Gets a value from the cache.
    ///
    /// See [`AssetCache::get_cached`] for more details.
    #[inline]
    pub fn get_cached<A: Storable>(self, id: &str) -> Option<Handle<'a, A>> {
        self.cache._get_cached(id)
    }

    /// Gets a value from the cache or inserts one.
    ///
    /// See [`AssetCache::get_or_insert`] for more details.
    #[inline]
    pub fn get_or_insert<A: Storable>(self, id: &str, default: A) -> Handle<'a, A> {
        self.cache._get_or_insert(id, default)
    }

    /// Returns `true` if the cache contains the specified asset.
    ///
    /// See [`AssetCache::contains`] for more details.
    #[inline]
    pub fn contains<A: Storable>(self, id: &str) -> bool {
        self.cache._contains::<A>(id)
    }

    /// Loads an asset.
    ///
    /// See [`AssetCache::load`] for more details.
    #[inline]
    pub fn load<A: Compound>(self, id: &str) -> Result<Handle<'a, A>, Error> {
        self.cache._load(id)
    }

    /// Loads an asset and panic if an error happens.
    ///
    /// See [`AssetCache::load_expect`] for more details.
    #[inline]
    pub fn load_expect<A: Compound>(self, id: &str) -> Handle<'a, A> {
        self.cache._load_expect(id)
    }

    /// Gets a directory from the cache.
    ///
    /// See [`AssetCache::get_cached_dir`] for more details.
    #[inline]
    pub fn get_cached_dir<A: DirLoadable>(
        self,
        id: &str,
        recursive: bool,
    ) -> Option<DirHandle<'a, A>> {
        self.cache._get_cached_dir(id, recursive)
    }

    /// Returns `true` if the cache contains the specified directory.
    ///
    /// See [`AssetCache::contains_dir`] for more details.
    #[inline]
    pub fn contains_dir<A: DirLoadable>(&self, id: &str, recursive: bool) -> bool {
        self.cache._contains_dir::<A>(id, recursive)
    }

    /// Loads all assets of a given type from a directory.
    ///
    /// See [`AssetCache::load_dir`] for more details.
    #[inline]
    pub fn load_dir<A: DirLoadable>(
        self,
        id: &str,
        recursive: bool,
    ) -> Result<DirHandle<'a, A>, Error> {
        self.cache._load_dir(id, recursive)
    }

    /// Loads an owned version of an asset.
    ///
    /// See [`AssetCache::load_owned`] for more details.
    #[inline]
    pub fn load_owned<A: Compound>(self, id: &str) -> Result<A, Error> {
        self.cache._load_owned(id)
    }

    /// Temporarily prevent `Compound` dependencies to be recorded.
    ///
    /// See [`AssetCache::no_record`] for more details.
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

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub(crate) fn record_load<A: Compound>(
        self,
        id: &SharedString,
    ) -> Result<(A, Dependencies), crate::BoxedError> {
        let (asset, records) = if let Some(reloader) = self.reloader() {
            records::record(reloader, || A::load(self, id))
        } else {
            (A::load(self, id), Dependencies::empty())
        };

        Ok((asset?, records))
    }

    #[inline]
    pub(crate) fn has_reloader(&self) -> bool {
        self.cache._has_reloader()
    }
}

impl fmt::Debug for AnyCache<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyCache").finish_non_exhaustive()
    }
}

pub(crate) trait Cache {
    #[cfg(feature = "hot-reloading")]
    fn reloader(&self) -> Option<&HotReloader>;

    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>>;

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()>;

    fn exists(&self, entry: DirEntry) -> bool;

    fn get_cached_entry_inner(&self, id: &str, typ: Type) -> Option<UntypedHandle>;

    fn contains(&self, id: &str, type_id: TypeId) -> bool;

    fn load_entry(&self, id: &str, typ: Type) -> Result<UntypedHandle, Error>;

    fn load_owned_entry(&self, id: &str, typ: Type) -> Result<CacheEntry, Error>;

    fn insert(&self, id: SharedString, type_id: TypeId, entry: CacheEntry) -> UntypedHandle;
}

pub(crate) trait RawCache: Sized {
    type Source: Source;

    fn assets(&self) -> &AssetMap;

    fn get_source(&self) -> &Self::Source;

    #[cfg(feature = "hot-reloading")]
    fn reloader(&self) -> Option<&HotReloader>;

    #[cold]
    fn add_asset(&self, id: &str, typ: Type) -> Result<UntypedHandle, Error> {
        log::trace!("Loading \"{}\"", id);

        let [id, id_clone] = SharedString::n_from_str(id);
        let cache = AnyCache { cache: self };
        let entry = crate::asset::load_and_record(cache, id_clone, typ)?;

        Ok(self.assets().insert(id, typ.type_id, entry))
    }
}

impl<T: RawCache> Cache for T {
    #[cfg(feature = "hot-reloading")]
    #[inline]
    fn reloader(&self) -> Option<&HotReloader> {
        self.reloader()
    }

    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>> {
        self.get_source().read(id, ext)
    }

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        self.get_source().read_dir(id, f)
    }

    fn exists(&self, entry: DirEntry) -> bool {
        self.get_source().exists(entry)
    }

    fn get_cached_entry_inner(&self, id: &str, typ: Type) -> Option<UntypedHandle> {
        #[cfg(feature = "hot-reloading")]
        if typ.is_hot_reloaded() {
            if let Some(reloader) = self.reloader() {
                let (id, entry) = match self.assets().get_entry(id, typ.type_id) {
                    Some((id, entry)) => (id, Some(entry)),
                    None => (id.into(), None),
                };
                records::add_record(reloader, id, typ.type_id);
                return entry;
            }
        }

        self.assets().get(id, typ.type_id)
    }

    #[inline]
    fn contains(&self, id: &str, type_id: TypeId) -> bool {
        self.assets().contains_key(id, type_id)
    }

    fn load_entry(&self, id: &str, typ: Type) -> Result<UntypedHandle, Error> {
        match self.get_cached_entry_inner(id, typ) {
            Some(entry) => Ok(entry),
            None => self.add_asset(id, typ),
        }
    }

    fn load_owned_entry(&self, id: &str, typ: Type) -> Result<CacheEntry, Error> {
        #[cfg(not(feature = "hot-reloading"))]
        let id = SharedString::from(id);

        #[cfg(feature = "hot-reloading")]
        let [id, id_clone] = SharedString::n_from_str(id);

        let asset = crate::asset::load_and_record(self._as_any_cache(), id, typ);

        #[cfg(feature = "hot-reloading")]
        if typ.is_hot_reloaded() {
            if let Some(reloader) = self.reloader() {
                records::add_record(reloader, id_clone, typ.type_id);
            }
        }

        asset
    }

    #[inline]
    fn insert(&self, id: SharedString, type_id: TypeId, entry: CacheEntry) -> UntypedHandle {
        self.assets().insert(id, type_id, entry)
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
    fn _get_cached<A: Storable>(&self, id: &str) -> Option<Handle<A>> {
        Some(self._get_cached_entry::<A>(id)?.downcast())
    }

    #[inline]
    fn _get_cached_entry<A: Storable>(&self, id: &str) -> Option<UntypedHandle> {
        self.get_cached_entry_inner(id, Type::of::<A>())
    }

    #[cold]
    fn add_any<A: Storable>(&self, id: &str, asset: A) -> UntypedHandle {
        let [id, id_clone] = SharedString::n_from_str(id);
        let entry = CacheEntry::new(asset, id_clone, || self._has_reloader());

        self.insert(id, TypeId::of::<A>(), entry)
    }

    fn _get_or_insert<A: Storable>(&self, id: &str, default: A) -> Handle<A> {
        let entry = match self._get_cached_entry::<A>(id) {
            Some(entry) => entry,
            None => self.add_any(id, default),
        };

        entry.downcast()
    }

    #[inline]
    fn _contains<A: Storable>(&self, id: &str) -> bool {
        self.contains(id, TypeId::of::<A>())
    }

    fn _load<A: Compound>(&self, id: &str) -> Result<Handle<A>, Error> {
        let entry = self.load_entry(id, Type::of::<A>())?;
        Ok(entry.downcast())
    }

    #[inline]
    #[track_caller]
    fn _load_expect<A: Compound>(&self, id: &str) -> Handle<A> {
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
    fn _get_cached_dir<A: DirLoadable>(&self, id: &str, recursive: bool) -> Option<DirHandle<A>> {
        Some(if recursive {
            let handle = self._get_cached(id)?;
            DirHandle::new_rec(handle, self._as_any_cache())
        } else {
            let handle = self._get_cached(id)?;
            DirHandle::new(handle, self._as_any_cache())
        })
    }

    #[inline]
    fn _load_dir<A: DirLoadable>(&self, id: &str, recursive: bool) -> Result<DirHandle<A>, Error> {
        Ok(if recursive {
            let handle = self._load(id)?;
            DirHandle::new_rec(handle, self._as_any_cache())
        } else {
            let handle = self._load(id)?;
            DirHandle::new(handle, self._as_any_cache())
        })
    }

    #[inline]
    fn _contains_dir<A: DirLoadable>(&self, id: &str, recursive: bool) -> bool {
        if recursive {
            self._contains::<crate::dirs::CachedRecDir<A>>(id)
        } else {
            self._contains::<crate::dirs::CachedDir<A>>(id)
        }
    }

    #[inline]
    fn _load_owned<A: Compound>(&self, id: &str) -> Result<A, Error> {
        let entry = self.load_owned_entry(id, Type::of::<A>())?;
        Ok(entry.into_inner().0)
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
