//! Generic asset loading definition
//!
//! See trait [`Loader`] for more informations
//!
//! [`Loader`]: trait.Loader.html

use std::{
    borrow::Cow,
    error::Error,
    marker::PhantomData,
    str::{self, FromStr},
};

/// Specifies how an asset is loaded.
///
/// With this trait, you can easily specify how you want your data to be loaded.
///
/// # Basic usage
///
/// Most of the time, you don't need to implement this trait yourself, as there
/// are implementations for the most formats (using `serde`). Don't forget to
/// enable the corresponding feature if needed !
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
///     const EXT: &'static str = "ron";
///
///     // Specify here how to convert raw data
///     type Loader = loader::RonLoader;
/// }
/// # }}
/// ```
pub trait Loader<T> {
    /// Loads an asset from its raw bytes representation.
    fn load(content: Cow<[u8]>) -> Result<T, Box<dyn Error + Send + Sync>>;
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
///     const EXT: &'static str = "ron";
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
    fn load(content: Cow<[u8]>) -> Result<T, Box<dyn Error + Send + Sync>> {
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
///     const EXT: &'static str = "ip";
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
    fn load(content: Cow<[u8]>) -> Result<T, Box<dyn Error + Send + Sync>> {
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
    fn load(content: Cow<[u8]>) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        Ok(content.into_owned())
    }
}

/// Loads assets as a String.
///
/// The file content is assumed to be valid UTF-8.
///
/// This Loader cannot be used to implement the Asset trait, but can be used by
/// [`LoadFrom`].
///
/// [`LoadFrom`]: struct.LoadFrom.html
#[derive(Debug)]
pub struct StringLoader;
impl Loader<String> for StringLoader {
    fn load(content: Cow<[u8]>) -> Result<String, Box<dyn Error + Send + Sync>> {
        Ok(String::from_utf8(content.into_owned())?)
    }
}

/// Loads assets that can be parsed with `FromStr`.
///
/// Do not use this loader to load `String`s, prefer using [`StringLoader`],
/// which is more efficient.
///
/// If you want your custom type to work with this loader, make sure that
/// `FromStr::Err` meet the requirements
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
    <T as FromStr>::Err: Error + Send + Sync + 'static,
{
    fn load(content: Cow<[u8]>) -> Result<T, Box<dyn Error + Send + Sync>> {
        let string = str::from_utf8(&content)?;
        Ok(string.parse()?)
    }
}

macro_rules! serde_loader {
    ($feature:literal, $doc:literal, $name:ident, $fun:path) => {
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
            fn load(content: Cow<[u8]>) -> Result<T, Box<dyn Error + Send + Sync>> {
                Ok($fun(&*content)?)
            }
        }

    }
}

serde_loader!("bincode", "Loads assets from Bincode encoded files.", BincodeLoader, serde_bincode::deserialize);
serde_loader!("cbor", "Loads assets from CBOR encoded files.", CborLoader, serde_cbor::from_slice);
serde_loader!("json", "Loads assets from JSON files.", JsonLoader, serde_json::from_slice);
serde_loader!("msgpack", "Loads assets from MessagePack files.", MessagePackLoader, serde_msgpack::decode::from_read);
serde_loader!("ron", "Loads assets from RON files.", RonLoader, serde_ron::de::from_bytes);
serde_loader!("toml", "Loads assets from TOML files.", TomlLoader, serde_toml::de::from_slice);
serde_loader!("yaml", "Loads assets from YAML files.", YamlLoader, serde_yaml::from_slice);
