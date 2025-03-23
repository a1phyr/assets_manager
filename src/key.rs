use std::{any::TypeId, fmt, hash};

use crate::{AssetCache, Compound, Error, SharedString, entry::CacheEntry};

impl Inner {
    fn of<T: Compound>() -> &'static Self {
        fn load<T: Compound>(cache: &AssetCache, id: SharedString) -> Result<CacheEntry, Error> {
            match T::load(cache, &id) {
                Ok(asset) => Ok(CacheEntry::new(asset, id, || cache.is_hot_reloaded())),
                Err(err) => Err(Error::new(id, err)),
            }
        }

        &Self {
            hot_reloaded: T::HOT_RELOADED,
            load: load::<T>,
        }
    }
}

#[allow(dead_code)]
pub(crate) struct Inner {
    pub hot_reloaded: bool,
    pub load: fn(&AssetCache, id: SharedString) -> Result<CacheEntry, Error>,
}

/// A structure to represent the type on an [`Asset`]
#[derive(Clone, Copy)]
pub(crate) struct Type {
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
            inner: Inner::of::<T>(),
        }
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
