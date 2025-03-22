#![allow(dead_code)]

use std::{
    any::{Any, TypeId},
    cmp, fmt, hash,
};

use crate::{AssetCache, Compound, Error, SharedString, asset::Storable, entry::CacheEntry};

impl Inner {
    fn of_asset<T: Compound>() -> &'static Self {
        fn load_entry<T: Compound>(
            cache: &AssetCache,
            id: SharedString,
        ) -> Result<CacheEntry, Error> {
            match T::load(cache, &id) {
                Ok(asset) => Ok(CacheEntry::new(asset, id, || cache.is_hot_reloaded())),
                Err(err) => Err(Error::new(id, err)),
            }
        }

        &Self {
            hot_reloaded: T::HOT_RELOADED,
            load: load_entry::<T>,
        }
    }

    #[allow(clippy::extra_unused_type_parameters)]
    fn of_any<T: Any>() -> &'static Self {
        fn load(_: &AssetCache, _: SharedString) -> Result<CacheEntry, Error> {
            panic!("Attempted to load non-`Compound` type")
        }

        &Self {
            hot_reloaded: false,
            load,
        }
    }
}

pub(crate) struct Inner {
    hot_reloaded: bool,
    pub load: fn(&AssetCache, id: SharedString) -> Result<CacheEntry, Error>,
}

/// A structure to represent the type on an [`Asset`]
#[derive(Clone, Copy)]
pub struct Type {
    // TODO: move this into `inner` when `TypeId::of` is const-stable
    pub(crate) type_id: TypeId,
    pub(crate) inner: &'static Inner,
}

impl Type {
    /// Creates an `AssetType` for type `T`.
    #[inline]
    pub(crate) fn of_asset<T: Compound>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            inner: Inner::of_asset::<T>(),
        }
    }

    #[inline]
    pub(crate) fn of_any<T: Storable>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            inner: Inner::of_any::<T>(),
        }
    }

    #[inline]
    pub fn is_hot_reloaded(self) -> bool {
        self.inner.hot_reloaded
    }
}

impl hash::Hash for Type {
    #[inline]
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
    }
}

impl PartialEq for Type {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
    }
}

impl Eq for Type {}

impl PartialOrd for Type {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Type {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.type_id.cmp(&other.type_id)
    }
}

impl fmt::Debug for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("AssetType")
            .field("type_id", &self.type_id)
            .finish()
    }
}

/// The key used to identify assets
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AssetKey {
    pub typ: Type,
    pub id: SharedString,
}

impl AssetKey {
    /// Creates a `OwnedKey` with the given type and id.
    #[allow(dead_code)]
    #[inline]
    pub fn new(id: SharedString, typ: Type) -> Self {
        Self { id, typ }
    }
}
