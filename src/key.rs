#![allow(dead_code)]

use std::{any::TypeId, cmp, fmt, hash};

use crate::{asset::Storable, entry::CacheEntry, utils, AnyCache, Compound, Error, SharedString};

impl Inner {
    fn of_asset<T: Compound>() -> &'static Self {
        fn load_entry<T: Compound>(cache: AnyCache, id: SharedString) -> Result<CacheEntry, Error> {
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

    fn of_storable<T: Storable>() -> &'static Self {
        fn load(_: AnyCache, _: SharedString) -> Result<CacheEntry, Error> {
            panic!("Attempted to load `Storable` type")
        }

        &Self {
            hot_reloaded: T::HOT_RELOADED,
            load,
        }
    }
}

pub(crate) struct Inner {
    hot_reloaded: bool,
    pub load: fn(AnyCache, id: SharedString) -> Result<CacheEntry, Error>,
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
    pub(crate) fn of_storable<T: Storable>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            inner: Inner::of_storable::<T>(),
        }
    }

    #[inline]
    pub fn of<T: Storable>() -> Self {
        T::get_type::<utils::Private>()
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
