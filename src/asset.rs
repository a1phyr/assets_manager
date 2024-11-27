//! Values loadable from a cache.
//!
//! # Asset kinds
//!
//! In `assets_manager`, assets are stored in an [`AssetCache`], and are usually
//! loaded from a [`Source`], a file system abstraction. Most of the I/O with
//! the source is handled by `assets_manager`, so you can focus on the rest.
//!
//! This crate defines several kinds of assets, that have different use cases:
//! - The most common, [`Asset`]s, that are loaded from a single file. An
//!   `Asset` gives a way to get a Rust value from raw bytes. This is nothing
//!   more than sugar on top of `Compound`.
//! - [`Compound`] are created by loading other assets and composing them. They
//!   can also read sources directly.
//! - [`Storable`] is the widest category: everything `'static` type can fit in
//!   it. Values of types that implement `Storable` can be inserted in a cache,
//!   but provide no way to construct them.
//!
//! Additionnally, [`DirLoadable`] assets can be loaded by directory, eventually
//! recursively. All `Asset` types implement this trait out of the box, but it
//! can be extended to work with any `Compound`, though it requires a custom
//! definition.
//!
//! # Hot-reloading
//!
//! Each asset is reloading when any file or directory it reads is modified, or
//! when a asset it depends on is reloaded itself.
//!
//! Additionally, one can explicitly disable hot-reloading for a type.
//!
//! Note that hot-reloading is not atomic: if asset `A` depends on `B`, you can
//! observe a state where `B` is reloaded but `A` is not reloaded yet.
//! Additionally, if `A` fails to reload, the inconsistent state is kept as is.

#[cfg(feature = "ab_glyph")]
mod fonts;
#[cfg(feature = "gltf")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf")))]
mod gltf;

#[cfg(test)]
mod tests;

pub use crate::dirs::DirLoadable;

#[allow(unused)]
use crate::{
    entry::CacheEntry,
    loader,
    source::Source,
    utils::{Private, SharedBytes, SharedString},
    AnyCache, AssetCache, BoxedError, Error,
};
use crate::{error::ErrorKind, key::Type, loader::Loader};

#[allow(unused)]
use std::{borrow::Cow, io, sync::Arc};

#[cfg(feature = "gltf")]
pub use self::gltf::Gltf;

#[cfg(doc)]
use crate::Handle;

/// An asset is a type loadable from raw bytes.
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
    /// If this array is empty, loading an asset of this type returns an error
    /// unless a default value is provided with the `default_value` method.
    const EXTENSIONS: &'static [&'static str] = &[Self::EXTENSION];

    /// Specifies a way to convert raw bytes into the asset.
    ///
    /// See module [`loader`] for implementations of common conversions.
    type Loader: Loader<Self>;

    /// Specifies a eventual default value to use if an asset fails to load. If
    /// this method returns `Ok`, the returned value is used as an asset. In
    /// particular, if this method always returns `Ok`, `AssetCache::load` is
    /// guaranteed not to fail.
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
    /// use assets_manager::{Asset, BoxedError, SharedString, loader};
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
    ///     fn default_value(id: &SharedString, error: BoxedError) -> Result<Item, BoxedError> {
    ///         eprintln!("Error loading {}: {}. Using default value", id, error);
    ///         Ok(Item::default())
    ///     }
    /// }
    /// # }}
    /// ```
    #[inline]
    #[allow(unused_variables)]
    fn default_value(id: &SharedString, error: BoxedError) -> Result<Self, BoxedError> {
        Err(error)
    }

    /// If `false`, disable hot-reloading for assets of this type (`true` by
    /// default). This avoids having to lock the asset to read it (ie it makes
    /// [`Handle::read`] a noop)
    const HOT_RELOADED: bool = true;
}

pub(crate) fn load_from_source<T: Asset>(
    source: impl Source,
    id: &SharedString,
) -> Result<T, BoxedError> {
    let load_with_ext = |ext| -> Result<T, ErrorKind> {
        let asset = source
            .read(id, ext)?
            .with_cow(|content| T::Loader::load(content, ext))?;
        Ok(asset)
    };

    let mut error = ErrorKind::NoDefaultValue;

    for ext in T::EXTENSIONS {
        match load_with_ext(ext) {
            Err(err) => error = err.or(error),
            Ok(asset) => return Ok(asset),
        }
    }

    T::default_value(id, error.into())
}

/// An asset type that can load other kinds of assets.
///
/// `Compound`s can be loaded and retrieved by an [`AssetCache`].
///
/// A `Compound` often needs to reference other assets, but `Compound` requires
/// `'static` and a `Handle` is borrowed. See [top-level documentation] for
/// workarounds.
///
/// [top-level documentation]: crate#getting-owned-data
///
/// Note that all [`Asset`]s implement `Compound`.
///
/// # Hot-reloading
///
/// Any asset loaded from the given cache is registered as a dependency of the
/// Compound. When the former is reloaded, the latter will be reloaded too. An
/// asset cannot depend on itself, or it may cause deadlocks to happen.
///
/// To opt out of dependencies recording, use [`AssetCache::no_record`].
pub trait Compound: Sized + Send + Sync + 'static {
    /// Loads an asset from the cache.
    ///
    /// This function should not perform any kind of I/O: such concern should be
    /// delegated to [`Asset`]s.
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError>;

    /// If `false`, disable hot-reloading for assets of this type (`true` by
    /// default). This avoids having to lock the asset to read it (ie it makes
    /// [`Handle::read`] a noop)
    const HOT_RELOADED: bool = true;
}

fn is_invalid_id(id: &str) -> bool {
    id.starts_with('.')
        || id.ends_with('.')
        || id.contains("..")
        || id.contains('/')
        || id.contains('\\')
}

#[inline]
pub(crate) fn load_and_record(
    cache: AnyCache,
    id: SharedString,
    typ: Type,
) -> Result<CacheEntry, Error> {
    if is_invalid_id(&id) {
        return Err(Error::new(id, ErrorKind::InvalidId.into()));
    }

    #[cfg(feature = "hot-reloading")]
    if typ.is_hot_reloaded() {
        if let Some(reloader) = cache.reloader() {
            let (entry, deps) = crate::hot_reloading::records::record(reloader, || {
                (typ.inner.load)(cache, id.clone())
            });
            if entry.is_ok() {
                reloader.add_asset(id, deps, typ);
            }
            return entry;
        }
    }

    (typ.inner.load)(cache, id)
}

impl<T> Compound for T
where
    T: Asset,
{
    #[inline]
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        load_from_source(cache.raw_source(), id)
    }

    const HOT_RELOADED: bool = Self::HOT_RELOADED;
}

impl<T> Compound for Arc<T>
where
    T: Compound,
{
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        let asset = T::load(cache, id)?;
        Ok(Arc::new(asset))
    }

    const HOT_RELOADED: bool = T::HOT_RELOADED;
}

/// Trait marker to store values in a cache.
///
/// This is the set of types that can be stored in a cache.
pub trait Storable: Sized + Send + Sync + 'static {}

impl<T> Storable for T where T: Send + Sync + 'static {}

macro_rules! string_assets {
    ( $( $typ:ty, )* ) => {
        $(
            impl Asset for $typ {
                const EXTENSION: &'static str = "txt";
                type Loader = loader::StringLoader;
            }
        )*
    }
}

string_assets! {
    String, Box<str>, SharedString,
}

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
            #[derive(Debug, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            #[repr(transparent)]
            pub struct $name<T>(pub T);

            #[cfg(feature = $feature)]
            impl<T> Clone for $name<T>
            where
                T: Clone
            {
                fn clone(&self) -> Self {
                    Self(self.0.clone())
                }

                fn clone_from(&mut self, other: &Self) {
                    self.0.clone_from(&other.0)
                }
            }

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
            impl<T> serde::Serialize for $name<T>
            where
                T: serde::Serialize,
            {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: serde::Serializer,
                {
                    self.0.serialize(serializer)
                }
            }

            #[cfg(feature = $feature)]
            impl<'de, T> serde::Deserialize<'de> for $name<T>
            where
                T: serde::Deserialize<'de>,
            {
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                where
                    D: serde::Deserializer<'de>,
                {
                    T::deserialize(deserializer).map($name)
                }

                fn deserialize_in_place<D>(deserializer: D, place: &mut Self) -> Result<(), D::Error>
                where
                    D: serde::Deserializer<'de>,
                {
                    T::deserialize_in_place(deserializer, &mut place.0)
                }
            }

            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
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

            #[cfg(feature = $feature)]
            impl<T> Default for $name<T>
            where
                T: Default,
            {
                #[inline]
                fn default() -> Self {
                    Self(T::default())
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

macro_rules! image_assets {
    (
        $(
            #[doc = $doc:literal]
            #[cfg(feature = $feature:literal)]
            struct $name:ident => (
                $format:path,
                [$($ext:literal),*],
            );
        )*
    ) => {
        $(
            #[doc = $doc]
            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            #[derive(Clone, Debug)]
            #[repr(transparent)]
            pub struct $name(pub image::DynamicImage);

            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            impl loader::Loader<$name> for loader::ImageLoader {
                #[inline]
                fn load(content: Cow<[u8]>, _: &str) -> Result<$name, BoxedError> {
                    let img = image::load_from_memory_with_format(&content, $format)?;
                    Ok($name(img))
                }
            }

            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            impl Asset for $name {
                const EXTENSIONS: &'static [&'static str] = &[$( $ext ),*];
                type Loader = loader::ImageLoader;
            }
        )*
    }
}

image_assets! {
    /// An asset to load BMP images.
    #[cfg(feature = "bmp")]
    struct Bmp => (
        image::ImageFormat::Bmp,
        ["bmp"],
    );

    /// An asset to load JPEG images.
    #[cfg(feature = "jpeg")]
    struct Jpeg => (
        image::ImageFormat::Jpeg,
        ["jpg", "jpeg"],
    );

    /// An asset to load PNG images.
    #[cfg(feature = "png")]
    struct Png => (
        image::ImageFormat::Png,
        ["png"],
    );

    /// An asset to load WebP images.
    #[cfg(feature = "webp")]
    struct Webp => (
        image::ImageFormat::WebP,
        ["webp"],
    );
}
