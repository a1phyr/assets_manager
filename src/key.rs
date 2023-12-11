#![allow(dead_code)]

use std::{any::TypeId, cmp, fmt, hash};

use crate::{
    asset::Storable, entry::CacheEntry, utils, AnyCache, Asset, Compound, Error, SharedString,
};

#[cfg(feature = "hot-reloading")]
use crate::hot_reloading::Dependencies;

pub(crate) struct AssetTypeInner {
    extensions: &'static [&'static str],
}

pub(crate) enum InnerType {
    Storable,
    Asset(AssetTypeInner),
    Compound,
}

impl Inner {
    fn of_asset<T: Asset>() -> &'static Self {
        &Self {
            hot_reloaded: T::HOT_RELOADED,
            load: T::_load_entry,
            typ: InnerType::Asset(AssetTypeInner {
                extensions: T::EXTENSIONS,
            }),
        }
    }

    fn of_compound<T: Compound>() -> &'static Self {
        &Self {
            hot_reloaded: T::HOT_RELOADED,
            load: T::_load_entry,
            typ: InnerType::Compound,
        }
    }

    fn of_storable<T: Storable>() -> &'static Self {
        fn load(_: AnyCache, _: SharedString) -> Result<CacheEntry, Error> {
            panic!("Attempted to load `Storable` type")
        }
        #[cfg(feature = "hot-reloading")]
        fn reload(_: AnyCache, _: SharedString) -> Option<Dependencies> {
            panic!("Attempted to load `Storable` type")
        }

        &Self {
            hot_reloaded: T::HOT_RELOADED,
            load,
            typ: InnerType::Storable,
        }
    }
}

pub(crate) struct Inner {
    hot_reloaded: bool,
    pub load: fn(AnyCache, id: SharedString) -> Result<CacheEntry, Error>,
    pub typ: InnerType,
}

/// A structure to represent the type on an [`Asset`]
#[derive(Clone, Copy)]
pub struct AssetType {
    // TODO: move this into `inner` when `TypeId::of` is const-stable
    pub(crate) type_id: TypeId,
    inner: &'static AssetTypeInner,
}

impl AssetType {
    /// Creates an `AssetType` for type `A`.
    #[inline]
    pub fn of<T: Asset>() -> Self {
        Type::of::<T>().to_asset_type().unwrap()
    }

    #[inline]
    pub(crate) fn new(type_id: TypeId, inner: &'static AssetTypeInner) -> Self {
        Self { type_id, inner }
    }

    /// The extensions associated with the reprensented asset type.
    #[inline]
    pub fn extensions(self) -> &'static [&'static str] {
        self.inner.extensions
    }
}

impl hash::Hash for AssetType {
    #[inline]
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
    }
}

impl PartialEq for AssetType {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
    }
}

impl Eq for AssetType {}

impl PartialOrd for AssetType {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AssetType {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.type_id.cmp(&other.type_id)
    }
}

impl fmt::Debug for AssetType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("AssetType")
            .field("type_id", &self.type_id)
            .finish()
    }
}

/// An untyped representation of a stored asset.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct AssetKey {
    /// The representation of the type of the asset.
    pub typ: AssetType,

    /// The id of the asset.
    pub id: SharedString,
}

impl AssetKey {
    /// Creates a new `AssetKey` from a type and an id.
    #[inline]
    pub fn new<A: Asset>(id: SharedString) -> Self {
        Self {
            id,
            typ: AssetType::of::<A>(),
        }
    }

    pub(crate) fn into_owned_key(self) -> utils::OwnedKey {
        utils::OwnedKey::new_with(self.id, self.typ.type_id)
    }
}

/// A structure to represent the type on an [`Asset`]
#[derive(Clone, Copy)]
pub struct Type {
    // TODO: move this into `inner` when `TypeId::of` is const-stable
    pub(crate) type_id: TypeId,
    pub(crate) inner: &'static Inner,
}

impl Type {
    /// Creates an `AssetType` for type `A`.
    #[inline]
    pub(crate) fn of_asset<A: Asset>() -> Self {
        Self {
            type_id: TypeId::of::<A>(),
            inner: Inner::of_asset::<A>(),
        }
    }

    #[inline]
    pub(crate) fn of_compound<A: Compound>() -> Self {
        Self {
            type_id: TypeId::of::<A>(),
            inner: Inner::of_compound::<A>(),
        }
    }

    #[inline]
    pub(crate) fn of_storable<A: Storable>() -> Self {
        Self {
            type_id: TypeId::of::<A>(),
            inner: Inner::of_storable::<A>(),
        }
    }

    #[inline]
    pub fn of<A: Storable>() -> Self {
        A::get_type::<utils::Private>()
    }

    #[inline]
    pub fn is_hot_reloaded(self) -> bool {
        self.inner.hot_reloaded
    }

    #[inline]
    pub fn to_asset_type(self) -> Option<AssetType> {
        match &self.inner.typ {
            InnerType::Asset(typ) => Some(AssetType {
                type_id: self.type_id,
                inner: typ,
            }),
            _ => None,
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
