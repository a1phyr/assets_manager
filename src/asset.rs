use crate::{
    AssetCache,
    Error,
    loader,
    source::Source,
    cache::load_from_source,
};

use std::sync::Arc;


/// An asset is a type loadable from a file.
///
/// `Asset`s can loaded and retreived by an [`AssetCache`].
///
/// # Extension
///
/// You can provide several extensions that will be used to search and load
/// assets. When loaded, each extension is tried in order until a file is
/// correctly loaded or no extension remain. The empty string `""` means a file
/// without extension. You cannot use character `.`.
///
/// The `EXTENSION` field is a convenient shortcut if your asset uses only one
/// extension. If you set a value for `EXTENSIONS` too, this field is ignored.
///
/// If neither `EXTENSION` nor `EXTENSIONS` is set, the default is no extension.
///
/// If you use hot-reloading, the asset will be reloaded each time one of the
/// file with the given extension is touched.
///
/// # Example
///
/// Suppose you make a physics simulutation, and you store positions and speeds
/// in a Bincode-encoded files, with extension ".data".
///
/// ```no_run
/// # cfg_if::cfg_if! { if #[cfg(feature = "bincode")] {
/// use assets_manager::{Asset, loader};
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Vector {
///     x: f32,
///     y: f32,
///     z: f32,
/// }
///
/// #[derive(Deserialize)]
/// struct World {
///     pos: Vec<Vector>,
///     speed: Vec<Vector>,
/// }
///
/// impl Asset for World {
///     const EXTENSION: &'static str = "data";
///     type Loader = loader::BincodeLoader;
/// }
/// # }}
/// ```
/// [`AssetCache`]: struct.AssetCache.html
pub trait Asset: Sized + Send + Sync + 'static {
    /// Use this field if your asset only uses one extension.
    ///
    /// This value is ignored if you set `EXTENSIONS` too.
    const EXTENSION: &'static str = "";

    /// This field enables you to specify multiple extension for an asset.
    ///
    /// If `EXTENSION` is provided, you don't have to set this constant.
    ///
    /// If this array is empty, loading an asset of this type returns
    /// [`Error::NoDefaultValue`] unless a default value is provided with the
    /// `default_value` method.
    ///
    /// [`Error::NoDefaultValue`]: enum.Error.html#variant.NoDefaultValue
    const EXTENSIONS: &'static [&'static str] = &[Self::EXTENSION];

    /// Specifies a way to to convert raw bytes into the asset.
    ///
    /// See module [`loader`] for implementations of common conversions.
    ///
    /// [`loader`]: loader/index.html
    type Loader: loader::Loader<Self>;

    /// Specifies a eventual default value to use if an asset fails to load. If
    /// this method returns `Ok`, the returned value is used as an asset. In
    /// particular, if this method always returns `Ok`, all `AssetCache::load*`
    /// (except `load_cached`) are guarantied not to fail.
    ///
    /// The `id` parameter is given to easily report the error.
    ///
    /// By default, this method always returns an error.
    ///
    /// # Example
    ///
    /// On error, log it and return a default value:
    ///
    /// ```no_run
    /// # cfg_if::cfg_if! { if #[cfg(feature = "json")] {
    /// use assets_manager::{Asset, Error, loader};
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, Default)]
    /// struct Item {
    ///     name: String,
    ///     kind: String,
    /// }
    ///
    /// impl Asset for Item {
    ///     const EXTENSION: &'static str = "json";
    ///     type Loader = loader::JsonLoader;
    ///
    ///     fn default_value(id: &str, error: Error) -> Result<Item, Error> {
    ///         eprintln!("Error loading {}: {}. Using default value", id, error);
    ///         Ok(Item::default())
    ///     }
    /// }
    /// # }}
    /// ```
    #[inline]
    #[allow(unused_variables)]
    fn default_value(id: &str, error: Error) -> Result<Self, Error> {
        Err(error)
    }
}

impl<A> Asset for Box<A>
where
    A: Asset,
{
    const EXTENSIONS: &'static [&'static str] = A::EXTENSIONS;
    type Loader = loader::LoadFromAsset<A>;

    #[inline]
    fn default_value(id: &str, error: Error) -> Result<Box<A>, Error> {
        A::default_value(id, error).map(Box::new)
    }
}

/// TODO
pub trait Compound: Sized + Send + Sync + 'static {
    /// TODO
    fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error>;

    #[doc(hidden)]
    #[cfg_attr(not(feature = "hot-reloading"), inline)]
    fn __private_load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        #[cfg(feature = "hot-reloading")]
        {
            use crate::utils::DepsRecord;

            let (asset, deps) = cache.record_load(id)?;
            cache.source().__private_hr_add_compound::<Self>(id, DepsRecord(deps));
            Ok(asset)
        }

        #[cfg(not(feature = "hot-reloading"))]
        { Self::load(cache, id) }
    }
}


impl<A: Asset> Compound for A {
    #[inline]
    fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        load_from_source(cache.source(), id)
    }

    #[cfg_attr(not(feature = "hot-reloading"), inline)]
    fn __private_load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        let asset = cache.no_record(|| Self::load(cache, id))?;

        #[cfg(feature = "hot-reloading")]
        cache.source().__private_hr_add_asset::<A>(id);

        Ok(asset)
    }
}

impl<A> Compound for Arc<A>
where
    A: Compound,
{
    fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        cache.load_owned::<A>(id).map(Arc::new)
    }
}
