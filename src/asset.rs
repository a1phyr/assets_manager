//! Values loadable from a cache.
//!
//! Assets are data that are used in a program. They are usually loaded from the
//! filesystem or from an archive.
//!
//! In `assets_manager`, assets are types that implement the [`Asset`] trait.
//! This trait specifies how to load an asset from an [`AssetCache`] given its
//! ID. The `AssetCache` gives access to other assets (eg your render pipeline
//! may want to get some shaders) and to a [`Source`] (to give you access to the
//! filesystem, a ZIP archive, or wherever you store your assets).
//!
//! Asset IDs are strings that represent paths using dots as separators (e.g.,
//! `"example.common.name"`). Unlike filesystem paths, they always use dots
//! regardless of the platform. File extensions are handled separately.
//!
//! The [`FileAsset`] trait provides an easy way to implement the `Asset` trait
//! for assets that can be loaded from a single file (eg an image, a sound,
//! etc). This module provides utilities to easily implement `FileAsset` for
//! common formats, such as JSON, TOML or RON.
//!
//! Additionally, [`DirLoadable`] assets can be loaded by directory. All types
//! implementing `FileAsset` also get this trait out of the box. It can be
//! implemented for any `Asset`, but it requires a manual implementation.
//!
//! # Hot-reloading
//!
//! If the `Source` supports hot-reloading, each asset is automatically reloaded
//! when any file, directory or asset it depends on is modified. Dependencies of
//! assets are automatically recorded when they are (re)loaded.
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

use crate::{
    AssetCache, BoxedError,
    error::ErrorKind,
    loader::{self, Loader},
    source::Source,
    utils::SharedString,
};
use std::{borrow::Cow, sync::Arc};

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
/// use assets_manager::{BoxedError, FileAsset};
/// use serde::Deserialize;
/// use std::borrow::Cow;
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
/// impl FileAsset for World {
///     const EXTENSION: &'static str = "data";
///
///     fn from_bytes(bytes: Cow<[u8]>) -> Result<Self, BoxedError> {
///         assets_manager::asset::load_bincode_standard(&bytes)
///     }
/// }
/// # }}
/// ```
#[deprecated = "use `FileAsset` instead"]
pub trait Asset: Storable {
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

/// An asset that can be loaded from a single file.
///
/// Implementing this trait provides an implementation of [`Asset`].
pub trait FileAsset: Storable {
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

    /// Creates a value of this type from raw bytes.
    fn from_bytes(bytes: Cow<[u8]>) -> Result<Self, BoxedError>;

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
    /// use assets_manager::{BoxedError, FileAsset, SharedString};
    /// use std::borrow::Cow;
    ///
    /// #[derive(serde::Deserialize, Default)]
    /// struct Item {
    ///     name: String,
    ///     kind: String,
    /// }
    ///
    /// impl FileAsset for Item {
    ///     const EXTENSION: &'static str = "json";
    ///
    ///     fn from_bytes(bytes: Cow<[u8]>) -> Result<Self, BoxedError> {
    ///         assets_manager::asset::load_json(&bytes)
    ///     }
    ///
    ///     fn default_value(id: &SharedString, error: BoxedError) -> Result<Item, BoxedError> {
    ///         log::warn!("Error loading {}: {}. Using default value", id, error);
    ///         Ok(Item::default())
    ///     }
    /// }
    /// # }}
    /// ```
    #[inline]
    #[expect(unused_variables)]
    fn default_value(id: &SharedString, error: BoxedError) -> Result<Self, BoxedError> {
        Err(error)
    }

    /// If `false`, disables hot-reloading of assets of this type (`true` by
    /// default).
    const HOT_RELOADED: bool = true;
}

/// Loads [`FileAsset`] types.
#[non_exhaustive]
#[allow(missing_debug_implementations)]
pub struct AssetLoader;

impl<T: FileAsset> loader::Loader<T> for AssetLoader {
    #[inline]
    fn load(bytes: Cow<[u8]>, _: &str) -> Result<T, BoxedError> {
        T::from_bytes(bytes)
    }
}

impl<T: FileAsset> Asset for T {
    const EXTENSION: &'static str = T::EXTENSION;
    const EXTENSIONS: &'static [&'static str] = T::EXTENSIONS;

    type Loader = AssetLoader;

    const HOT_RELOADED: bool = T::HOT_RELOADED;
}

/// An asset type that can load other kinds of assets.
///
/// `Asset`s can be loaded and retrieved by an [`AssetCache`].
///
/// Note that all [`FileAsset`]s implement `Compound`.
pub trait Compound: Storable {
    /// Loads an asset from the cache.
    ///
    /// The cache gives access to its underlying [`Source`].
    ///
    /// # Hot-reloading
    ///
    /// Any file, directory or asset loaded from `cache` is registered as a
    /// dependency. When a dependency is modified (through direct modification
    /// or hot-reloading), the asset will be reloaded.
    ///
    /// If you don't use threads in this method, you don't need to write
    /// hot-reloading-specific code.
    ///
    /// An asset cannot depend on itself.
    ///
    /// To opt out of dependencies recording, use [`AssetCache::no_record`].
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError>;

    /// If `false`, disable hot-reloading for assets of this type (`true` by
    /// default). This avoids having to lock the asset to read it (ie it makes
    /// [`Handle::read`] a noop)
    const HOT_RELOADED: bool = true;
}

impl<T> Compound for T
where
    T: Asset,
{
    #[inline]
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
        let source = cache.source();

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

    const HOT_RELOADED: bool = Self::HOT_RELOADED;
}

impl<T> Compound for Arc<T>
where
    T: Compound,
{
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
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

#[inline]
fn cow_bytes_to_str(bytes: Cow<[u8]>) -> Result<Cow<str>, std::str::Utf8Error> {
    Ok(match bytes {
        Cow::Borrowed(b) => Cow::Borrowed(std::str::from_utf8(b)?),
        Cow::Owned(b) => Cow::Owned(String::from_utf8(b).map_err(|e| e.utf8_error())?),
    })
}

macro_rules! string_assets {
    ( $( $typ:ty, )* ) => {
        $(
            impl FileAsset for $typ {
                const EXTENSION: &'static str = "txt";

                fn from_bytes(bytes: Cow<[u8]>) -> Result<Self, BoxedError> {
                    Ok(cow_bytes_to_str(bytes)?.into())
                }
            }
        )*
    }
}

string_assets! {
    String, Box<str>, SharedString, Arc<str>,
}

/// Deserializes a value from a bincode-encoded file.
///
/// This function uses the standard bincode format, which is the default in bincode 2.0.
#[cfg(feature = "bincode")]
#[cfg_attr(docsrs, doc(cfg(feature = "bincode")))]
pub fn load_bincode_standard<'de, T: serde::Deserialize<'de>>(
    bytes: &'de [u8],
) -> Result<T, BoxedError> {
    let (res, _) = bincode::serde::borrow_decode_from_slice(bytes, bincode::config::standard())?;
    Ok(res)
}

/// Deserializes a value from a bincode-encoded file.
///
/// This function uses the legacy bincode format, which was the default in bincode 1.0.
#[cfg(feature = "bincode")]
#[cfg_attr(docsrs, doc(cfg(feature = "bincode")))]
pub fn load_bincode_legacy<'de, T: serde::Deserialize<'de>>(
    bytes: &'de [u8],
) -> Result<T, BoxedError> {
    let (res, _) = bincode::serde::borrow_decode_from_slice(bytes, bincode::config::legacy())?;
    Ok(res)
}

/// Deserializes a value from a JSON file.
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub fn load_json<'de, T: serde::Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, BoxedError> {
    serde_json::from_slice(bytes).map_err(Box::from)
}

/// Deserializes a value from a msgpack-encoded file.
#[cfg(feature = "msgpack")]
#[cfg_attr(docsrs, doc(cfg(feature = "msgpack")))]
pub fn load_msgpack<'de, T: serde::Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, BoxedError> {
    rmp_serde::from_slice(bytes).map_err(Box::from)
}

/// Deserializes a value from a RON file.
#[cfg(feature = "ron")]
#[cfg_attr(docsrs, doc(cfg(feature = "ron")))]
pub fn load_ron<'de, T: serde::Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, BoxedError> {
    ron::de::from_bytes(bytes).map_err(Box::from)
}

/// Deserializes a value from a TOML file.
#[cfg(feature = "toml")]
#[cfg_attr(docsrs, doc(cfg(feature = "toml")))]
pub fn load_toml<'de, T: serde::Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, BoxedError> {
    toml::from_slice(bytes).map_err(Box::from)
}

/// Deserializes a value from a YAML file.
#[cfg(feature = "yaml")]
#[cfg_attr(docsrs, doc(cfg(feature = "yaml")))]
pub fn load_yaml<'de, T: serde::Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, BoxedError> {
    serde_yaml::from_slice(bytes).map_err(Box::from)
}

/// Deserializes a value from text.
///
/// Leading and trailing whitespaces are trimmed from the input before
/// processing.
pub fn load_text<T>(bytes: &[u8]) -> Result<T, BoxedError>
where
    T: std::str::FromStr,
    T::Err: Into<BoxedError>,
{
    let str = std::str::from_utf8(bytes)?;
    str.trim().parse().map_err(Into::into)
}

macro_rules! serde_assets {
    (
        $(
            #[doc = $doc:literal]
            #[cfg(feature = $feature:literal)]
            struct $name:ident => (
                [$($ext:literal),*],
                $load:expr,
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
            impl<T> FileAsset for $name<T>
            where
                T: for<'de> serde::Deserialize<'de> + Send + Sync + 'static,
            {
                const EXTENSIONS: &'static [&'static str] = &[$( $ext ),*];

                fn from_bytes(bytes: Cow<[u8]>) -> Result<Self, BoxedError> {
                    $load(&*bytes).map(Self)
                }
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
        ["json"],
        load_json,
    );

    /// Loads a value from a JSON file.
    #[cfg(feature = "ron")]
    struct Ron => (
        ["ron"],
        load_ron,
    );

    /// Loads a value from a TOML file.
    #[cfg(feature = "toml")]
    struct Toml => (
        ["toml"],
        load_toml,
    );

    /// Loads a value from a YAML file.
    #[cfg(feature = "yaml")]
    struct Yaml => (
        ["yaml", "yml"],
        load_yaml,
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
            impl FileAsset for $name {
                const EXTENSIONS: &'static [&'static str] = &[$( $ext ),*];

                fn from_bytes(data: Cow<[u8]>) -> Result<Self, BoxedError> {
                    let img = image::load_from_memory_with_format(&data, $format)?;
                    Ok(Self(img))
                }
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
