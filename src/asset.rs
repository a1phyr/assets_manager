//! Values loadable from a cache.

use crate::{
    AssetCache,
    Error,
    loader,
    cache::load_from_source,
    source::Source,
    utils::PrivateMarker,
};

#[cfg(feature = "serde")]
#[allow(unused)]
use serde::{Deserialize, Serialize};

use std::sync::Arc;


/// An asset is a type loadable from a file.
///
/// `Asset`s can be loaded and retrieved by an [`AssetCache`].
///
/// This trait should only perform a conversion from raw bytes to the concrete
/// type. If you need to load other assets, please use the [`Compound`] trait.
///
/// # Extension
///
/// You can provide several extensions that will be used to search and load
/// assets. When loaded, each extension is tried in order until a file is
/// correctly loaded or no extension remains. The empty string `""` means a file
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
/// Suppose you make a physics simulation, and you store positions and speeds
/// in a Bincode-encoded file, with extension ".data".
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
    const EXTENSIONS: &'static [&'static str] = &[Self::EXTENSION];

    /// Specifies a way to convert raw bytes into the asset.
    ///
    /// See module [`loader`] for implementations of common conversions.
    type Loader: loader::Loader<Self>;

    /// Specifies a eventual default value to use if an asset fails to load. If
    /// this method returns `Ok`, the returned value is used as an asset. In
    /// particular, if this method always returns `Ok`, all `AssetCache::load*`
    /// (except `load_cached`) are guaranteed not to fail.
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

    /// If `false`, disable hot-reloading for assets of this type (`true` by
    /// default). If so, you may want to implement [`NotHotReloaded`] for this
    /// type to enable additional functions.
    const HOT_RELOADED: bool = true;
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

/// An asset type that can load other kinds of assets.
///
/// `Compound`s can be loaded and retrieved by an [`AssetCache`].
///
/// # Hot-reloading
///
/// Any asset loaded from the given cache is registered as a dependency of the
/// Compound. When the former is reloaded, the latter will be reloaded too. An
/// asset cannot depend on itself, or it may cause deadlocks to happen.
///
/// To opt out of dependencies recording, use `AssetCache::no_record`.
///
/// Note that directories are not considered as dependencies at the moment, but
/// this will come in a future (breaking) release.
pub trait Compound: Sized + Send + Sync + 'static {
    /// Loads an asset from the cache.
    ///
    /// This function should not perform any kind of I/O: such concern should be
    /// delegated to [`Asset`]s.
    fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error>;

    /// Loads an asset and registers it for hot-reloading if necessary.
    #[doc(hidden)]
    #[cfg_attr(not(feature = "hot-reloading"), inline)]
    fn _load<S: Source, P: PrivateMarker>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        #[cfg(feature = "hot-reloading")]
        {
            use crate::utils::DepsRecord;

            if Self::HOT_RELOADED {
                let (asset, deps) = cache.record_load(id)?;
                cache.source()._add_compound::<Self, P>(id, DepsRecord(deps));
                Ok(asset)
            } else {
                cache.no_record(|| Self::load(cache, id))
            }
        }

        #[cfg(not(feature = "hot-reloading"))]
        { Self::load(cache, id) }
    }

    /// If `false`, disable hot-reloading for assets of this type (`true` by
    /// default). If so, you may want to implement [`NotHotReloaded`] for this
    /// type to enable additional functions.
    const HOT_RELOADED: bool = true;

    #[doc(hidden)]
    /// Compile-time check that HOT_RELOADED is false when `NotHotReloaded` is
    /// implemented.
    /// ```compile_fail
    /// use assets_manager::{Asset, asset::NotHotReloaded, AssetCache, loader};
    ///
    /// struct A(i32);
    /// impl From<i32> for A {
    ///     fn from(x: i32) -> A { A(x) }
    /// }
    ///
    /// impl Asset for A {
    ///     type Loader = loader::LoadFrom<i32, loader::ParseLoader>;
    /// }
    /// impl NotHotReloaded for A {}
    ///
    /// let cache = AssetCache::new("assets")?;
    /// let handle = cache.load::<A>("tests")?;
    /// let _ = handle.get();
    /// # Ok::<(), assets_manager::BoxedError>(())
    /// ```
    const _CHECK_NOT_HOT_RELOADED: () = [()][Self::HOT_RELOADED as usize];
}


impl<A> Compound for A
where
    A: Asset,
{
    #[inline]
    fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        load_from_source(cache.source(), id)
    }

    #[cfg_attr(not(feature = "hot-reloading"), inline)]
    fn _load<S: Source, P: PrivateMarker>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        let asset = cache.no_record(|| Self::load(cache, id))?;

        #[cfg(feature = "hot-reloading")]
        if A::HOT_RELOADED {
            cache.source()._add_asset::<A, P>(id);
        }

        Ok(asset)
    }

    const HOT_RELOADED: bool = Self::HOT_RELOADED;
}

impl<A> Compound for Arc<A>
where
    A: Compound,
{
    fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        cache.load_owned::<A>(id).map(Arc::new)
    }
}


/// Mark an asset as not being hot-reloaded.
///
/// At the moment, the only use of this trait is to enable `Handle::get` for
/// types that implement it.
///
/// If you implement this trait, you MUST set [`Asset::HOT_RELOADED`] (or
/// [`Compound::HOT_RELOADED`] to `true` or you will get compile-error at best
/// and panics at worst. This is a workaround about Rust's type system current
/// limitations.
pub trait NotHotReloaded: Compound {}


macro_rules! serde_assets {
    (
        $(
            #[doc = $doc:literal]
            #[cfg(feature = $feature:literal)]
            struct $name:ident => (
                $loader:path,
                [$($ext:literal),*],
            );
        )*
    ) => {
        $(
            #[doc = $doc]
            ///
            /// This type can directly be used as an [`Asset`] to load values
            /// from an [`AssetCache`]. This is useful to load assets external
            /// types without a newtype wrapper (eg [`Vec`]).
            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
            #[serde(transparent)]
            pub struct $name<T>(pub T);

            #[cfg(feature = $feature)]
            impl<T> From<T> for $name<T> {
                #[inline]
                fn from(t: T) -> Self {
                    Self(t)
                }
            }

            #[cfg(feature = $feature)]
            impl<T> $name<T> {
                /// Unwraps the inner value.
                #[inline]
                pub fn into_inner(self) -> T {
                    self.0
                }
            }

            #[cfg(feature = $feature)]
            impl<T> Asset for $name<T>
            where
                T: for<'de> serde::Deserialize<'de> + Send + Sync + 'static,
            {
                const EXTENSIONS: &'static [&'static str] = &[$( $ext ),*];
                type Loader = loader::LoadFrom<T, $loader>;
            }

            #[cfg(feature = $feature)]
            impl<T> AsRef<T> for $name<T> {
                #[inline]
                fn as_ref(&self) -> &T {
                    &self.0
                }
            }
        )*
    }
}

serde_assets! {
    /// Loads a value from a RON file.
    #[cfg(feature = "json")]
    struct Json => (
        loader::JsonLoader,
        ["json"],
    );

    /// Loads a value from a JSON file.
    #[cfg(feature = "ron")]
    struct Ron => (
        loader::RonLoader,
        ["ron"],
    );

    /// Loads a value from a TOML file.
    #[cfg(feature = "toml")]
    struct Toml => (
        loader::TomlLoader,
        ["toml"],
    );

    /// Loads a value from a YAML file.
    #[cfg(feature = "yaml")]
    struct Yaml => (
        loader::YamlLoader,
        ["yaml", "yml"],
    );
}
