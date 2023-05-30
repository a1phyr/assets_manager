#![allow(dead_code)]

use std::{any::TypeId, cmp, fmt, hash};

use crate::{
    asset::Storable, entry::CacheEntry, utils, AnyCache, Asset, Compound, Error, SharedString,
};

#[cfg(feature = "hot-reloading")]
use crate::{
    cache::load_from_source, entry::UntypedHandle, hot_reloading::Dependencies, source::Source,
};

#[cfg(feature = "hot-reloading")]
pub(crate) trait AnyAsset: Send + Sync + 'static {
    fn reload(self: Box<Self>, entry: UntypedHandle);
    fn create(self: Box<Self>, id: SharedString) -> CacheEntry;
}

#[cfg(feature = "hot-reloading")]
impl<A: Asset> AnyAsset for A {
    fn reload(self: Box<Self>, entry: UntypedHandle) {
        entry.downcast().write(*self);
    }

    fn create(self: Box<Self>, id: SharedString) -> CacheEntry {
        CacheEntry::new(*self, id, || true)
    }
}

#[cfg(feature = "hot-reloading")]
fn reload<T: Compound>(cache: AnyCache, id: SharedString) -> Option<Dependencies> {
    // Outline these functions to reduce the amount of monomorphized code
    fn log_ok(id: SharedString) {
        log::info!("Reloading \"{id}\"");
    }
    fn log_err(err: Error) {
        log::warn!("Error reloading \"{}\": {}", err.id(), err.reason());
    }

    let handle = cache.get_cached::<T>(&id)?;
    let typ = Type::of::<T>();
    let load_fn = || (typ.inner.load)(cache, id);

    let (entry, deps) = if let Some(reloader) = cache.reloader() {
        crate::hot_reloading::records::record(reloader, load_fn)
    } else {
        (load_fn(), Dependencies::empty())
    };

    match entry {
        Ok(entry) => {
            let (asset, id) = entry.into_inner();
            handle.write(asset);
            log_ok(id);
            Some(deps)
        }
        Err(err) => {
            log_err(err);
            None
        }
    }
}

pub(crate) struct AssetTypeInner {
    extensions: &'static [&'static str],

    #[cfg(feature = "hot-reloading")]
    #[allow(clippy::type_complexity)]
    pub load_from_source: fn(&dyn Source, id: &SharedString) -> Result<Box<dyn AnyAsset>, Error>,
}

pub(crate) struct CompoundTypeInner {
    #[cfg(feature = "hot-reloading")]
    pub reload: crate::hot_reloading::ReloadFn,
}

pub(crate) enum InnerType {
    Storable,
    Asset(AssetTypeInner),
    Compound(CompoundTypeInner),
}

impl Inner {
    fn of_asset<T: Asset>() -> &'static Self {
        #[cfg(feature = "hot-reloading")]
        fn load<A: Asset>(
            source: &dyn Source,
            id: &SharedString,
        ) -> Result<Box<dyn AnyAsset>, Error> {
            let asset = load_from_source::<A>(source, id)?;
            Ok(Box::new(asset))
        }

        &Self {
            hot_reloaded: T::HOT_RELOADED,
            load: T::_load_entry,
            typ: InnerType::Asset(AssetTypeInner {
                extensions: T::EXTENSIONS,
                #[cfg(feature = "hot-reloading")]
                load_from_source: load::<T>,
            }),
        }
    }

    fn of_compound<T: Compound>() -> &'static Self {
        &Self {
            hot_reloaded: T::HOT_RELOADED,
            load: T::_load_entry,
            typ: InnerType::Compound(CompoundTypeInner {
                #[cfg(feature = "hot-reloading")]
                reload: reload::<T>,
            }),
        }
    }

    fn of_storable<T: Storable>() -> &'static Self {
        fn load(_: AnyCache, _: SharedString) -> Result<CacheEntry, Error> {
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
    type_id: TypeId,
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

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn load_from_source<S: Source>(
        self,
        source: &S,
        id: &SharedString,
    ) -> Result<Box<dyn AnyAsset>, Error> {
        (self.inner.load_from_source)(source, id)
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
        self.type_id.partial_cmp(&other.type_id)
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
