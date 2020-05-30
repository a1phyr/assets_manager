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

#[allow(unused_imports)]
use std::{
    borrow::Cow,
    convert::Infallible,
    error::Error,
    fmt::Display,
    io,
    marker::PhantomData,
    str::{self, FromStr},
};

mod errors;
pub use errors::{StringLoaderError, ParseLoaderError};

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
/// ## Example
///
/// ```
/// use assets_manager::loader::Loader;
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
///     type Err = Box<dyn Error>;
///
///     fn load(content: io::Result<Cow<[u8]>>) -> Result<Fruit, Self::Err> {
///         match str::from_utf8(&content?)?.trim() {
///             "apple" => Ok(Fruit::Apple),
///             "banana" => Ok(Fruit::Banana),
///             "pear" => Ok(Fruit::Pear),
///             _ => Err("Invalid fruit".into()),
///         }
///     }
/// }
///
/// # let fruit = Ok(b" banana \n"[..].into());
/// # assert_eq!(FruitLoader::load(fruit).unwrap(), Fruit::Banana);
/// ```

pub trait Loader<T> {
    /// The associated error which can be returned from loading.
    ///
    /// For a quick implementation you can use `Box<dyn Error>`.
    type Err: Display;

    /// Loads an asset from its raw bytes representation.
    fn load(content: io::Result<Cow<[u8]>>) -> Result<T, Self::Err>;
}

/// Returns the default value in case of failure.
///
/// If the inner loader returns an error, the default value of `T` will be
/// provided instead.
///
/// # Example
///
/// ```
/// # cfg_if::cfg_if! { if #[cfg(feature = "ron")] {
/// use serde::Deserialize;
/// use assets_manager::{Asset, loader::{RonLoader, LoadOrDefault}};
///
/// #[derive(Default, Deserialize)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
///
/// impl Asset for Point {
///     const EXTENSION: &'static str = "ron";
///     type Loader = LoadOrDefault<RonLoader>;
/// }
/// # }}
/// ```
#[derive(Debug)]
pub struct LoadOrDefault<L>(PhantomData<L>);
impl<T, L> Loader<T> for LoadOrDefault<L>
where
    T: Default,
    L: Loader<T>,
{
    type Err = Infallible;

    fn load(content: io::Result<Cow<[u8]>>) -> Result<T, Self::Err> {
        L::load(content).or_else(|_| Ok(T::default()))
    }
}

/// Load assets from another type.
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
    type Err = L::Err;

    fn load(content: io::Result<Cow<[u8]>>) -> Result<T, Self::Err> {
        Ok(L::load(content)?.into())
    }
}

/// Loads assets as a `Vec<u8>`.
///
/// This Loader cannot be used to implement the Asset trait, but can be used by
/// [`LoadFrom`].
///
/// [`LoadFrom`]: struct.LoadFrom.html
#[derive(Debug)]
pub struct BytesLoader;
impl Loader<Vec<u8>> for BytesLoader {
    type Err = io::Error;

    fn load(content: io::Result<Cow<[u8]>>) -> Result<Vec<u8>, Self::Err> {
        Ok(content?.into_owned())
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
    type Err = StringLoaderError;

    fn load(content: io::Result<Cow<[u8]>>) -> Result<String, Self::Err> {
        Ok(String::from_utf8(content?.into_owned())?)
    }
}

/// Loads assets that can be parsed with `FromStr`.
///
/// Do not use this loader to load `String`s, prefer using [`StringLoader`],
/// which is more efficient.
///
/// If you want your custom type to work with this loader, make sure that
/// `FromStr::Err` meets the requirement.
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
    <T as FromStr>::Err: Display,
{
    type Err = ParseLoaderError<<T as FromStr>::Err>;

    fn load(content: io::Result<Cow<[u8]>>) -> Result<T, Self::Err> {
        str::from_utf8(&content?)?.parse().map_err(ParseLoaderError::Parse)
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
            type Err = $error;

            #[inline]
            fn load(content: io::Result<Cow<[u8]>>) -> Result<T, Self::Err> {
                Ok($fun(&*content?)?)
            }
        }

    }
}

serde_loader!("bincode", "Loads assets from Bincode encoded files.", BincodeLoader, serde_bincode::deserialize, serde_bincode::Error);
serde_loader!("cbor", "Loads assets from CBOR encoded files.", CborLoader, serde_cbor::from_slice, serde_cbor::Error);
serde_loader!("json", "Loads assets from JSON files.", JsonLoader, serde_json::from_slice, Box<dyn Error>);
serde_loader!("msgpack", "Loads assets from MessagePack files.", MessagePackLoader, serde_msgpack::decode::from_read, Box<dyn Error>);
serde_loader!("ron", "Loads assets from RON files.", RonLoader, serde_ron::de::from_bytes, serde_ron::de::Error);
serde_loader!("toml", "Loads assets from TOML files.", TomlLoader, serde_toml::de::from_slice, Box<dyn Error>);
serde_loader!("yaml", "Loads assets from YAML files.", YamlLoader, serde_yaml::from_slice, Box<dyn Error>);
