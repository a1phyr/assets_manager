//! Generic asset loading definition
//!
//! This module defines a trait [`Loader`], to specify how [assets] are loaded
//! from the filesystem.

//! It also defines loaders, ie types that implement [`Loader`], so in most
//! cases you don't have to implement this trait yourself. These loaders work
//! with standard traits and `serde`.
//!
//! See trait [`Loader`] for more informations.
//!
//! [assets]: ../trait.Asset.html
//! [`Loader`]: trait.Loader.html

use crate::BoxedError;

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
/// This trait is a little complex, but it makes it quite powerful.
///
/// Function `load` does the conversion between raw bytes and the concrete Rust
/// value. It takes the result of the file loading as parameter, so it is up to
/// the loader to handle an eventual I/O error. If no I/O error happen, bytes
/// are given as a `Cow<[u8]>` to avoid unnecessary clones.
///
/// The extension used to load the asset is also passed as parameter, which can
/// be useful if an asset type uses several extensions.
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
    fn load(content: Cow<[u8]>, ext: &str) -> Result<T, BoxedError>;
}


/// Loads assets from another type.
///
/// An example case for this is to easily load wrapper types, which is needed
/// if the wrapped type is defined in another crate.
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
///
/// [`LoadFrom`]: struct.LoadFrom.html
#[derive(Debug)]
pub struct BytesLoader;
impl Loader<Vec<u8>> for BytesLoader {
    fn load(content: Cow<[u8]>, _: &str) -> Result<Vec<u8>, BoxedError> {
        Ok(content.into_owned())
    }
}
impl Loader<Box<[u8]>> for BytesLoader {
    fn load(content: Cow<[u8]>, _: &str) -> Result<Box<[u8]>, BoxedError> {
        Ok(content.into())
    }
}

/// Loads assets as a String.
///
/// The file content is parsed as UTF-8.
///
/// This Loader cannot be used to implement the Asset trait, but can be used by
/// [`LoadFrom`].
///
/// [`LoadFrom`]: struct.LoadFrom.html
#[derive(Debug)]
pub struct StringLoader;
impl Loader<String> for StringLoader {
    fn load(content: Cow<[u8]>, _: &str) -> Result<String, BoxedError> {
        Ok(String::from_utf8(content.into_owned())?)
    }
}
impl Loader<Box<str>> for StringLoader {
    fn load(content: Cow<[u8]>, ext: &str) -> Result<Box<str>, BoxedError> {
        StringLoader::load(content, ext).map(String::into_boxed_str)
    }
}

/// Loads assets that can be parsed with `FromStr`.
///
/// Do not use this loader to load `String`s, prefer using [`StringLoader`],
/// which is more efficient.
///
/// If you want your custom type to work with this loader, make sure that
/// `FromStr::Error` meets the requirement.
///
/// See trait [`Loader`] for more informations.
///
/// [`StringLoader`]: struct.StringLoader.html
/// [`Loader`]: trait.Loader.html
#[derive(Debug)]
pub struct ParseLoader;
impl<T> Loader<T> for ParseLoader
where
    T: FromStr,
    BoxedError: From<<T as FromStr>::Err>
{
    fn load(content: Cow<[u8]>, _: &str) -> Result<T, BoxedError> {
        Ok(str::from_utf8(&content)?.parse()?)
    }
}

macro_rules! serde_loader {
    ($feature:literal, $doc:literal, $name:ident, $fun:path, $error:ty) => {
        #[doc = $doc]
        ///
        /// See trait [`Loader`] for more informations.
        ///
        /// [`Loader`]: trait.Loader.html
        #[cfg(feature = $feature)]
        #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
        #[derive(Debug)]
        pub struct $name;

        #[cfg(feature = $feature)]
        impl<T> Loader<T> for $name
        where
            T: for<'de> serde::Deserialize<'de>,
        {
            #[inline]
            fn load(content: Cow<[u8]>, _: &str) -> Result<T, BoxedError> {
                Ok($fun(&*content)?)
            }
        }

    }
}

serde_loader!("bincode", "Loads assets from Bincode encoded files.", BincodeLoader, serde_bincode::deserialize, serde_bincode::Error);
serde_loader!("cbor", "Loads assets from CBOR encoded files.", CborLoader, serde_cbor::from_slice, serde_cbor::Error);
serde_loader!("json", "Loads assets from JSON files.", JsonLoader, serde_json::from_slice, serde_json::Error);
serde_loader!("msgpack", "Loads assets from MessagePack files.", MessagePackLoader, serde_msgpack::decode::from_read, serde_msgpack::decode::Error);
serde_loader!("ron", "Loads assets from RON files.", RonLoader, serde_ron::de::from_bytes, serde_ron::de::Error);
serde_loader!("toml", "Loads assets from TOML files.", TomlLoader, serde_toml::de::from_slice, serde_toml::de::Error);
serde_loader!("yaml", "Loads assets from YAML files.", YamlLoader, serde_yaml::from_slice, serde_yaml::Error);
