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
//!   `Asset` gives a way to get a Rust value from raw bytes.
//! - [`Compound`] are created by loading other assets and composing them.
//! - [`Storable`] is the widest category: everything `'static` type can fit in
//!   it. Values of types that implement `Storable` can be inserted in a cache,
//!   but provide no way to construct them.
//!
//! Additionnally, [`DirLoadable`] assets can be loaded by directory, eventually
//! recursively. All `Asset` types implement this trait out of the box, but it
//! can be extended to work with `Compound`s.
//!
//! # Hot-reloading
//!
//! Different asset kinds have different interactions with hot-reloading:
//! - `Asset`s are reloaded when the file they were loaded from is edited.
//! - `Compound`s are reloaded when any asset they depend on to build themselves
//!   is reloaded.
//! - Directories are never reloaded (note that individual assets in a directory
//!   are still reloaded).
//!
//! Additionally, one can explicitly disable hot-reloading for a type.

#[cfg(feature = "ab_glyph")]
mod fonts;
#[cfg(feature = "gltf")]
mod gltf;

#[cfg(test)]
mod tests;

pub use crate::dirs::DirLoadable;

use crate::key::Type;
#[allow(unused)]
use crate::{
    cache::load_from_source,
    entry::CacheEntry,
    loader,
    source::Source,
    utils::{PrivateMarker, SharedBytes, SharedString},
    AnyCache, AssetCache, BoxedError, Error,
};

#[cfg(feature = "rodio")]
#[allow(unused)]
use rodio::decoder::{Decoder, DecoderError};

#[cfg(feature = "serde")]
#[allow(unused)]
use serde::{Deserialize, Serialize};

#[allow(unused)]
use std::{borrow::Cow, io, sync::Arc};

#[cfg(feature = "gltf")]
pub use self::gltf::Gltf;

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
    type Loader: loader::Loader<Self>;

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

    const HOT_RELOADED: bool = A::HOT_RELOADED;
}

impl<A> NotHotReloaded for Box<A> where A: Asset + NotHotReloaded {}

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

    #[doc(hidden)]
    fn _load_entry(cache: AnyCache, id: SharedString) -> Result<CacheEntry, Error> {
        match Self::load(cache, &id) {
            Ok(asset) => Ok(CacheEntry::new(asset, id, || cache.is_hot_reloaded())),
            Err(err) => Err(Error::new(id, err)),
        }
    }

    /// If `false`, disable hot-reloading for assets of this type (`true` by
    /// default). If so, you may want to implement [`NotHotReloaded`] for this
    /// type to enable additional functions.
    const HOT_RELOADED: bool = true;

    #[doc(hidden)]
    #[inline]
    fn get_type<P: PrivateMarker>() -> crate::key::Type {
        crate::key::Type::of_compound::<Self>()
    }
}

#[inline]
pub(crate) fn load_and_record(
    cache: AnyCache,
    id: SharedString,
    typ: Type,
) -> Result<CacheEntry, Error> {
    #[cfg(feature = "hot-reloading")]
    if typ.is_hot_reloaded() {
        if let Some(reloader) = cache.reloader() {
            match &typ.inner.typ {
                crate::key::InnerType::Storable => (),
                crate::key::InnerType::Asset(inner) => {
                    let asset = (typ.inner.load)(cache, id.clone())?;
                    reloader.add_asset(id, crate::key::AssetType::new(typ.type_id, inner));
                    return Ok(asset);
                }
                crate::key::InnerType::Compound(inner) => {
                    let (entry, deps) = crate::hot_reloading::records::record(reloader, || {
                        (typ.inner.load)(cache, id.clone())
                    });
                    let entry = entry?;
                    reloader.add_compound(id, deps, typ, inner.reload);
                    return Ok(entry);
                }
            }
        }
    }

    (typ.inner.load)(cache, id)
}

impl<A> Compound for A
where
    A: Asset,
{
    #[inline]
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        Ok(load_from_source(&cache.raw_source(), id)?)
    }

    #[doc(hidden)]
    fn _load_entry(cache: AnyCache, id: SharedString) -> Result<CacheEntry, Error> {
        let asset: Self = load_from_source(&cache.raw_source(), &id)?;
        Ok(CacheEntry::new(asset, id, || cache.is_hot_reloaded()))
    }

    const HOT_RELOADED: bool = Self::HOT_RELOADED;

    #[doc(hidden)]
    #[inline]
    fn get_type<P: PrivateMarker>() -> crate::key::Type {
        crate::key::Type::of_asset::<Self>()
    }
}

impl<A> Compound for Arc<A>
where
    A: Compound,
{
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        let asset = cache.load_owned::<A>(id)?;
        Ok(Arc::new(asset))
    }

    const HOT_RELOADED: bool = A::HOT_RELOADED;
}

impl<A> NotHotReloaded for Arc<A> where A: Compound + NotHotReloaded {}

/// Mark a type as not being hot-reloaded.
///
/// At the moment, the only use of this trait is to enable `Handle::get` for
/// types that implement it.
///
/// If you implement this trait on a type that also implement [`Asset`] or
/// [`Compound`], you MUST set [`Asset::HOT_RELOADED`] (or
/// [`Compound::HOT_RELOADED`] to `true` or you will get compile-error at best
/// and panics at worst.
///
/// On a type that implements [`Storable`] directly, you can implement this
/// trait wihout issues.
///
/// This trait is a workaround about Rust's type system current limitations.
pub trait NotHotReloaded: Storable {}

/// Trait marker to store values in a cache.
///
/// Implementing this trait is necessary to use [`AssetCache::get_cached`]. This
/// trait is already implemented for all `Compound` types.
///
/// This trait is a workaround about Rust's current lack of specialization.
pub trait Storable: Sized + Send + Sync + 'static {
    #[doc(hidden)]
    const HOT_RELOADED: bool = false;

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
    const _CHECK_NOT_HOT_RELOADED: () = assert!(!Self::HOT_RELOADED);

    #[doc(hidden)]
    #[inline]
    fn get_type<P: PrivateMarker>() -> crate::key::Type {
        crate::key::Type::of_storable::<Self>()
    }
}

impl<A> Storable for A
where
    A: Compound,
{
    #[doc(hidden)]
    const HOT_RELOADED: bool = A::HOT_RELOADED;

    #[inline]
    fn get_type<P: PrivateMarker>() -> crate::key::Type {
        Self::get_type::<P>()
    }
}

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

macro_rules! impl_storable {
    ( $( $typ:ty, )* ) => {
        $(
            impl Storable for $typ {}
            impl NotHotReloaded for $typ {}
        )*
    }
}

impl_storable! {
    i8, i16, i32, i64, i128, isize,
    u8, u16, u32, u64, u128, usize,
    f32, f64, char, &'static str,
    SharedBytes,
}

impl<A: Send + Sync + 'static> Storable for Vec<A> {}
impl<A: Send + Sync + 'static> NotHotReloaded for Vec<A> {}
impl<A: Send + Sync + 'static> Storable for &'static [A] {}
impl<A: Send + Sync + 'static> NotHotReloaded for &'static [A] {}

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
            #[derive(Debug, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
            #[serde(transparent)]
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

macro_rules! sound_assets {
    (
        $(
            #[doc = $doc:literal]
            #[cfg(feature = $feature:literal)]
            struct $name:ident => (
                $decoder:path,
                [$($ext:literal),*],
            );
        )*
    ) => {
        $(
            #[doc = $doc]
            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            #[derive(Clone, Debug)]
            pub struct $name(SharedBytes);

            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            impl loader::Loader<$name> for loader::SoundLoader {
                #[inline]
                fn load(content: Cow<[u8]>, _: &str) -> Result<$name, BoxedError> {
                    let bytes = content.into();
                    Ok($name::new(bytes)?)
                }
            }

            #[cfg(feature = $feature)]
            #[cfg_attr(docsrs, doc(cfg(feature = $feature)))]
            impl Asset for $name {
                const EXTENSIONS: &'static [&'static str] = &[$( $ext ),*];
                type Loader = loader::SoundLoader;
            }

            #[cfg(feature = $feature)]
            impl $name {
                /// Creates a new sound from raw bytes.
                #[inline]
                pub fn new(bytes: SharedBytes) -> Result<$name, DecoderError> {
                    // We have to clone the bytes here because `Decoder::new`
                    // requires a 'static lifetime, but it should be cheap
                    // anyway.
                    let _ = $decoder(io::Cursor::new(bytes.clone()))?;
                    Ok($name(bytes))
                }

                /// Creates a [`Decoder`] that can be send to `rodio` to play
                /// sounds.
                #[inline]
                pub fn decoder(self) -> Decoder<io::Cursor<SharedBytes>> {
                    $decoder(io::Cursor::new(self.0)).unwrap()
                }

                #[inline]
                /// Returns a bytes slice of the sound content.
                pub fn as_bytes(&self) -> &[u8] {
                    &self.0
                }

                /// Convert the sound back to raw bytes.
                #[inline]
                pub fn into_bytes(self) -> SharedBytes {
                    self.0
                }
            }

            #[cfg(feature = $feature)]
            impl AsRef<[u8]> for $name {
                fn as_ref(&self) -> &[u8] {
                    &self.0
                }
            }
        )*
    }
}

sound_assets! {
    /// Load FLAC sounds
    #[cfg(feature = "flac")]
    struct Flac => (
        Decoder::new_flac,
        ["flac"],
    );

    /// Load MP3 sounds
    #[cfg(feature = "mp3")]
    struct Mp3 => (
        Decoder::new_mp3,
        ["mp3"],
    );

    /// Load Vorbis sounds
    #[cfg(feature = "vorbis")]
    struct Vorbis => (
        Decoder::new_vorbis,
        ["ogg"],
    );

    /// Load WAV sounds
    #[cfg(feature = "wav")]
    struct Wav => (
        Decoder::new_wav,
        ["wav"],
    );
}
