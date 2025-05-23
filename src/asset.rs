//! Values loadable from a cache.
//!
//! # Asset kinds
//!
//! In `assets_manager`, assets are stored in an [`AssetCache`], and are usually
//! loaded from a [`Source`], a file system abstraction. Most of the I/O with
//! the source is handled by `assets_manager`, so you can focus on the rest.
//!
//! This crate defines several kinds of assets, that have different use cases:
//! - The most common, [`FileAsset`]s, that are loaded from a single file. A
//!   `FileAsset` gives a way to get a Rust value from raw bytes. This is nothing
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
    AssetCache, BoxedError, Error,
    entry::CacheEntry,
    error::ErrorKind,
    source::Source,
    utils::{Private, SharedBytes, SharedString},
};

#[allow(unused)]
use std::{borrow::Cow, io, sync::Arc};

#[cfg(feature = "gltf")]
pub use self::gltf::Gltf;

#[cfg(doc)]
use crate::Handle;

/// An asset that can be loaded from a single file.
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
    /// use serde::Deserialize;
    /// use std::borrow::Cow;
    ///
    /// #[derive(Deserialize, Default)]
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
    /// default).
    const HOT_RELOADED: bool = true;
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
/// Note that all [`FileAsset`]s implement `Compound`.
///
/// # Hot-reloading
///
/// Any asset loaded from the given cache is registered as a dependency of the
/// Compound. When the former is reloaded, the latter will be reloaded too. An
/// asset cannot depend on itself, or it may cause deadlocks to happen.
///
/// To opt out of dependencies recording, use [`AssetCache::no_record`].
pub trait Asset: Storable {
    /// Loads an asset from the cache.
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError>;

    /// If `false`, disable hot-reloading for assets of this type (`true` by
    /// default). This avoids having to lock the asset to read it (ie it makes
    /// [`Handle::read`] a noop)
    const HOT_RELOADED: bool = true;
}

/// Deprecated trait alias for [`Asset`].
#[deprecated = "Use `Asset` instead"]
pub trait Compound: Asset {
    /// Loads an asset from the cache.
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError>;

    /// If `false`, disable hot-reloading for assets of this type (`true` by
    /// default). This avoids having to lock the asset to read it (ie it makes
    /// [`Handle::read`] a noop)
    const HOT_RELOADED: bool = true;
}

#[allow(deprecated)]
impl<T> Compound for T
where
    T: FileAsset,
{
    #[inline]
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
        let source = cache.source();

        let load_with_ext = |ext| -> Result<T, ErrorKind> {
            let asset = source
                .read(id, ext)?
                .with_cow(|content| T::from_bytes(content))?;
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

#[allow(deprecated)]
impl<T: Compound> Asset for T {
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
        <T as Compound>::load(cache, id)
    }

    const HOT_RELOADED: bool = <T as Compound>::HOT_RELOADED;
}

impl<T> Asset for Arc<T>
where
    T: Asset,
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
    basic_toml::from_slice(bytes).map_err(Box::from)
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
            /// This type can directly be used as a [`FileAsset`] to load values
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
