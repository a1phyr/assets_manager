//! Generic asset loading definition
//!
//! This module defines a trait [`Loader`], to specify how [assets] are loaded
//! from raw bytes.
//!
//! It also defines loaders, ie types that implement [`Loader`], so in most
//! cases you don't have to implement this trait yourself. These loaders work
//! with standard traits and `serde`.
//!
//! See trait [`Loader`] for more information.
//!
//! [assets]: `crate::Asset`

use crate::{BoxedError, SharedBytes, SharedString};

use std::{
    borrow::Cow,
    marker::PhantomData,
    str::{self, FromStr},
};

#[cfg(test)]
mod tests;

/// Specifies how an asset is loaded.
///
/// With this trait, you can easily specify how you want your data to be loaded.
/// This trait is generic, so the same `Loader` type can be used to load several
/// asset types.
///
/// # Basic usage
///
/// Most of the time, you don't need to implement this trait yourself, or even
/// care about the definition, as there are implementations for common formats
/// and conversions. Don't forget to enable the corresponding feature if needed !
///
/// ## Example
///
/// ```no_run
/// # cfg_if::cfg_if! { if #[cfg(feature = "ron")] {
/// use serde::Deserialize;
/// use assets_manager::{Asset, loader};
///
/// // The struct you want to load
/// #[derive(Deserialize)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
///
/// impl Asset for Point {
///     const EXTENSION: &'static str = "ron";
///
///     // Specify here how to convert raw data
///     type Loader = loader::RonLoader;
/// }
/// # }}
/// ```
///
/// # Implementing `Loader`
///
/// Function `load` does the conversion from raw bytes to the concrete Rust
/// value.
///
/// ## Example
///
/// ```
/// use assets_manager::{Asset, BoxedError, loader::Loader};
/// use std::{borrow::Cow, error::Error, io, str};
///
/// # #[derive(PartialEq, Eq, Debug)]
/// enum Fruit {
///     Apple,
///     Banana,
///     Pear,
/// }
///
/// struct FruitLoader;
/// impl Loader<Fruit> for FruitLoader {
///     fn load(content: Cow<[u8]>, _: &str) -> Result<Fruit, BoxedError> {
///         match str::from_utf8(&content)?.trim() {
///             "apple" => Ok(Fruit::Apple),
///             "banana" => Ok(Fruit::Banana),
///             "pear" => Ok(Fruit::Pear),
///             _ => Err("Invalid fruit".into()),
///         }
///     }
/// }
///
/// impl Asset for Fruit {
///     const EXTENSION: &'static str = "txt";
///     type Loader = FruitLoader;
/// }
///
/// # let fruit = b" banana \n"[..].into();
/// # assert_eq!(FruitLoader::load(fruit, "").unwrap(), Fruit::Banana);
/// ```

pub trait Loader<T> {
    /// Loads an asset from its raw bytes representation.
    ///
    /// The extension used to load the asset is also passed as parameter, which can
    /// be useful to guess the format if an asset type uses several extensions.
    fn load(content: Cow<[u8]>, ext: &str) -> Result<T, BoxedError>;
}

/// Loads assets from another type.
///
/// An example case for this is to easily load wrapper types, which are needed
/// when the wrapped type is defined in another crate.
///
/// # Example
///
/// ```
/// use assets_manager::{Asset, loader::{LoadFrom, ParseLoader}};
/// use std::net::IpAddr;
///
/// struct Ip(IpAddr);
///
/// impl From<IpAddr> for Ip {
///     fn from(ip: IpAddr) -> Ip {
///         Ip(ip)
///     }
/// }
///
/// impl Asset for Ip {
///     const EXTENSION: &'static str = "ip";
///     type Loader = LoadFrom<IpAddr, ParseLoader>;
/// }
/// ```
#[derive(Debug)]
pub struct LoadFrom<U, L>(PhantomData<(U, L)>);
impl<T, U, L> Loader<T> for LoadFrom<U, L>
where
    U: Into<T>,
    L: Loader<U>,
{
    #[inline]
    fn load(content: Cow<[u8]>, ext: &str) -> Result<T, BoxedError> {
        Ok(L::load(content, ext)?.into())
    }
}

/// Loads assets from another asset.
pub type LoadFromAsset<A> = LoadFrom<A, <A as crate::Asset>::Loader>;

/// Loads assets as raw bytes.
///
/// This Loader cannot be used to implement the Asset trait, but can be used by
/// [`LoadFrom`].
#[derive(Debug)]
pub struct BytesLoader(());
impl Loader<Vec<u8>> for BytesLoader {
    #[inline]
    fn load(content: Cow<[u8]>, _: &str) -> Result<Vec<u8>, BoxedError> {
        Ok(content.into_owned())
    }
}
impl Loader<Box<[u8]>> for BytesLoader {
    #[inline]
    fn load(content: Cow<[u8]>, _: &str) -> Result<Box<[u8]>, BoxedError> {
        Ok(content.into())
    }
}
impl Loader<SharedBytes> for BytesLoader {
    #[inline]
    fn load(content: Cow<[u8]>, _: &str) -> Result<SharedBytes, BoxedError> {
        Ok(content.into())
    }
}

/// Loads assets as a String.
///
/// The file content is parsed as UTF-8.
///
/// This Loader cannot be used to implement the Asset trait, but can be used by
/// [`LoadFrom`].
#[derive(Debug)]
pub struct StringLoader(());
impl Loader<String> for StringLoader {
    #[inline]
    fn load(content: Cow<[u8]>, _: &str) -> Result<String, BoxedError> {
        Ok(String::from_utf8(content.into_owned())?)
    }
}
impl Loader<Box<str>> for StringLoader {
    #[inline]
    fn load(content: Cow<[u8]>, ext: &str) -> Result<Box<str>, BoxedError> {
        StringLoader::load(content, ext).map(String::into_boxed_str)
    }
}
impl Loader<SharedString> for StringLoader {
    #[inline]
    fn load(content: Cow<[u8]>, _: &str) -> Result<SharedString, BoxedError> {
        Ok(match content {
            Cow::Owned(o) => String::from_utf8(o)?.into(),
            Cow::Borrowed(b) => str::from_utf8(b)?.into(),
        })
    }
}

/// Loads assets that can be parsed with [`FromStr`].
///
/// Leading and trailing whitespaces are removed from the input before
/// processing.
///
/// Do not use this loader to load `String`s, prefer using [`StringLoader`],
/// which is generally more efficient and does not trim whitespaces.
///
/// If you want your custom type to work with this loader, make sure that
/// `FromStr::Err` meets the requirement.
///
/// See trait [`Loader`] for more informations.
#[derive(Debug)]
pub struct ParseLoader(());
impl<T> Loader<T> for ParseLoader
where
    T: FromStr,
    BoxedError: From<<T as FromStr>::Err>,
{
    #[inline]
    fn load(content: Cow<[u8]>, _: &str) -> Result<T, BoxedError> {
        Ok(str::from_utf8(&content)?.trim().parse()?)
    }
}

/// Loads assets used as sounds.
#[derive(Debug)]
pub struct SoundLoader(());

/// Loads assets as images.
#[derive(Debug)]
pub struct ImageLoader(());

#[cfg(feature = "image")]
#[cfg_attr(docsrs, doc(cfg(feature = "image")))]
impl Loader<image::DynamicImage> for ImageLoader {
    fn load(content: Cow<[u8]>, ext: &str) -> Result<image::DynamicImage, BoxedError> {
        Ok(match image::ImageFormat::from_extension(ext) {
            Some(format) => image::load_from_memory_with_format(&content, format)?,
            None => image::load_from_memory(&content)?,
        })
    }
}

/// Loads glTF assets.
#[derive(Debug)]
pub struct GltfLoader(());

#[cfg(feature = "gltf")]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf")))]
impl Loader<gltf::Gltf> for GltfLoader {
    fn load(content: Cow<[u8]>, _: &str) -> Result<gltf::Gltf, BoxedError> {
        Ok(gltf::Gltf::from_slice(&content)?)
    }
}

/// Loads fonts.
#[derive(Debug)]
pub struct FontLoader(());

macro_rules! serde_loaders {
    (
        $(
            #[doc = $doc:literal]
            #[cfg(feature = $feature:literal)]
            struct $name:ident => $fun:expr;
        )*
    ) => {
        $(
            #[doc = $doc]
            ///
            /// See trait [`Loader`] for more informations.
            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            #[derive(Debug)]
            pub struct $name(());

            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            impl<T> Loader<T> for $name
            where
                T: for<'de> serde::Deserialize<'de>,
            {
                #[inline]
                fn load(content: Cow<[u8]>, _: &str) -> Result<T, BoxedError> {
                    Ok($fun(&*content)?)
                }
            }
        )*
    }
}

serde_loaders! {
    /// Loads assets from Bincode encoded files.
    #[cfg(feature = "bincode")]
    struct BincodeLoader => bincode::deserialize;

    /// Loads assets from JSON files.
    #[cfg(feature = "json")]
    struct JsonLoader => serde_json::from_slice;

    /// Loads assets from MessagePack files.
    #[cfg(feature = "msgpack")]
    struct MessagePackLoader => rmp_serde::from_slice;

    /// Loads assets from RON files.
    #[cfg(feature = "ron")]
    struct RonLoader => ron::de::from_bytes;

    /// Loads assets from TOML files.
    #[cfg(feature = "toml")]
    struct TomlLoader => toml_edit::de::from_slice;

    /// Loads assets from YAML files.
    #[cfg(feature = "yaml")]
    struct YamlLoader => serde_yaml::from_slice;
}
