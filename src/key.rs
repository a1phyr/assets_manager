#![allow(dead_code)]

use std::{any::TypeId, cmp, fmt, hash};

use crate::{
    cache::load_from_source,
    entry::{CacheEntry, CacheEntryInner},
    source::Source,
    utils, Asset, Error, SharedString,
};

pub(crate) trait AnyAsset: Send + Sync + 'static {
    fn reload(self: Box<Self>, entry: CacheEntryInner);
    fn create(self: Box<Self>, id: SharedString) -> CacheEntry;
}

impl<A: Asset> AnyAsset for A {
    fn reload(self: Box<Self>, entry: CacheEntryInner) {
        entry.handle::<A>().as_dynamic().write(*self);
    }

    fn create(self: Box<Self>, id: SharedString) -> CacheEntry {
        CacheEntry::new::<A>(*self, id)
    }
}

fn load<A: Asset>(source: &dyn Source, id: &str) -> Result<Box<dyn AnyAsset>, Error> {
    let asset = load_from_source::<A, _>(source, id)?;
    Ok(Box::new(asset))
}

struct Inner {
    extensions: &'static [&'static str],
    #[allow(clippy::type_complexity)]
    load: fn(&dyn Source, id: &str) -> Result<Box<dyn AnyAsset>, Error>,
}

impl Inner {
    fn of<A: Asset>() -> &'static Self {
        &Inner {
            extensions: A::EXTENSIONS,
            load: load::<A>,
        }
    }
}

/// A structure to represent the type on an [`Asset`]
#[derive(Clone, Copy)]
pub struct AssetType {
    // TODO: move this into `inner` when `TypeId::of` is const-stable
    type_id: TypeId,
    inner: &'static Inner,
}

impl AssetType {
    /// Creates an `AssetType` for type `A`.
    #[inline]
    pub fn of<A: Asset>() -> Self {
        Self {
            type_id: TypeId::of::<A>(),
            inner: Inner::of::<A>(),
        }
    }

    /// The extensions associated with the reprensented asset type.
    #[inline]
    pub fn extensions(self) -> &'static [&'static str] {
        self.inner.extensions
    }

    pub(crate) fn load<S: Source>(self, source: &S, id: &str) -> Result<Box<dyn AnyAsset>, Error> {
        (self.inner.load)(source, id)
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
        self.type_id.partial_cmp(&other.type_id)
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
