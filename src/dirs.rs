use crate::{
    Asset, AssetCache, BoxedError, Error, FileAsset, Handle, SharedString, Storable,
    source::{DirEntry, Source},
};

use std::{fmt, io, marker::PhantomData};

/// Assets that are loadable from directories
///
/// Types that implement this trait can be used with [`AssetCache::load_dir`] to
/// load all available assets in a directory (eventually recursively).
///
/// This trait is automatically implemented for all types that implement
/// [`FileAsset`], and you can implement it to extend your own `Asset`s.
///
/// # Exemple implementation
///
/// Imagine you have several playlists with a JSON manifest to specify the ids
/// of the musics to include.
///
/// ```no_run
/// # cfg_if::cfg_if! { if #[cfg(feature = "json")] {
/// use assets_manager::{
///     AssetCache, Asset, BoxedError, FileAsset, SharedString,
///     asset::{DirLoadable, Json},
///     source::{DirEntry, Source},
/// };
///
/// /// A music for our game.
/// #[derive(Clone)]
/// struct Music {
///     /* ... */
/// }
///
/// impl FileAsset for Music {
///     /* ... */
/// #   const EXTENSION: &'static str = "ogg";
/// #   fn from_bytes(_: std::borrow::Cow<'_, [u8]>) -> Result<Self, BoxedError> { todo!() }
/// }
///
/// /// A simple playlist, an ordered list of musics.
/// struct Playlist {
///     sounds: Vec<Music>,
/// }
///
/// // Specify how to load a playlist
/// impl Asset for Playlist {
///     fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
///         // Read the manifest (a list of ids)
///         let manifest = cache.load::<Json<Vec<String>>>(id)?.read();
///
///         // Load each sound
///         let sounds = manifest.0.iter()
///             .map(|id| Ok(cache.load::<Music>(id)?.cloned()))
///             .collect::<Result<_, BoxedError>>()?;
///
///         Ok(Playlist { sounds })
///     }
/// }
///
/// // Specify how to get ids of playlists in a directory
/// impl DirLoadable for Playlist {
///     fn select_ids(cache: &AssetCache, id: &SharedString) -> std::io::Result<Vec<SharedString>> {
///         let mut ids = Vec::new();
///
///         // Select all files with "json" extension (manifest files)
///         cache.source().read_dir(id, &mut |entry| {
///             if let DirEntry::File(id, ext) = entry {
///                 if ext == "json" {
///                     ids.push(id.into());
///                 }
///             }
///         })?;
///
///         Ok(ids)
///     }
/// }
/// # }}
/// ```
pub trait DirLoadable: Storable {
    /// Returns the ids of the assets contained in the directory given by `id`.
    ///
    /// Note that the order of the returned ids is not kept, and that redundant
    /// ids are removed.
    fn select_ids(cache: &AssetCache, id: &SharedString) -> io::Result<Vec<SharedString>>;

    /// Executes the given closure for each id of a child directory of the given
    /// directory. The default implementation reads the cache's source.
    #[inline]
    fn sub_directories(
        cache: &AssetCache,
        id: &SharedString,
        mut f: impl FnMut(&str),
    ) -> io::Result<()> {
        cache.source().read_dir(id, &mut |entry| {
            if let DirEntry::Directory(id) = entry {
                f(id);
            }
        })
    }
}

impl<T> DirLoadable for T
where
    T: FileAsset,
{
    #[inline]
    fn select_ids(cache: &AssetCache, id: &SharedString) -> io::Result<Vec<SharedString>> {
        fn inner(
            cache: &AssetCache,
            id: &str,
            extensions: &[&str],
        ) -> io::Result<Vec<SharedString>> {
            let mut ids = Vec::new();

            // Select all files with an extension valid for type `T`
            cache.source().read_dir(id, &mut |entry| {
                if let DirEntry::File(id, ext) = entry {
                    if extensions.contains(&ext) {
                        ids.push(id.into());
                    }
                }
            })?;

            Ok(ids)
        }

        inner(cache, id, T::EXTENSIONS)
    }
}

impl<T> DirLoadable for std::sync::Arc<T>
where
    T: DirLoadable,
{
    #[inline]
    fn select_ids(cache: &AssetCache, id: &SharedString) -> io::Result<Vec<SharedString>> {
        T::select_ids(cache, id)
    }

    #[inline]
    fn sub_directories(
        cache: &AssetCache,
        id: &SharedString,
        f: impl FnMut(&str),
    ) -> io::Result<()> {
        T::sub_directories(cache, id, f)
    }
}

/// Stores ids in a directory containing assets of type `T`
pub struct RawDirectory<T> {
    ids: Vec<SharedString>,
    _marker: PhantomData<T>,
}

impl<T> Asset for RawDirectory<T>
where
    T: DirLoadable,
{
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
        let mut ids = T::select_ids(cache, id)?;

        // Remove duplicated entries
        ids.sort_unstable();
        ids.dedup();

        Ok(RawDirectory {
            ids,
            _marker: PhantomData,
        })
    }

    const HOT_RELOADED: bool = true;
}

impl<T> RawDirectory<T> {
    /// Returns an iterator over the ids of the assets in the directory.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = &SharedString> {
        self.ids.iter()
    }
}

impl<T> RawDirectory<T>
where
    T: Storable,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This fonction does not do any I/O and assets that previously failed to
    /// load are ignored.
    #[inline]
    pub fn iter_cached<'h, 'a: 'h>(
        &'h self,
        cache: &'a AssetCache,
    ) -> impl Iterator<Item = &'a Handle<T>> + 'h {
        self.ids().filter_map(move |id| cache.get(id))
    }
}

impl<T> RawDirectory<T>
where
    T: Asset,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This function will happily try to load all assets, even if an error
    /// occured the last time it was tried.
    #[inline]
    pub fn iter<'h, 'a: 'h>(
        &'h self,
        cache: &'a AssetCache,
    ) -> impl ExactSizeIterator<Item = Result<&'a Handle<T>, Error>> + 'h {
        self.ids().map(move |id| cache.load(id))
    }
}

impl<T> fmt::Debug for RawDirectory<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawDirectory")
            .field("ids", &self.ids)
            .finish()
    }
}

/// Stores ids in a recursive directory containing assets of type `T`
pub struct RawRecursiveDirectory<T> {
    ids: Vec<SharedString>,
    _marker: PhantomData<T>,
}

impl<T> Asset for RawRecursiveDirectory<T>
where
    T: DirLoadable,
{
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
        // Load the current directory
        let this = cache.load::<RawDirectory<T>>(id)?;
        let mut ids = this.read().ids.clone();

        // Recursively load child directories
        T::sub_directories(cache, id, |id| {
            if let Ok(child) = cache.load::<RawRecursiveDirectory<T>>(id) {
                ids.extend_from_slice(&child.read().ids);
            }
        })?;

        Ok(RawRecursiveDirectory {
            ids,
            _marker: PhantomData,
        })
    }

    const HOT_RELOADED: bool = true;
}

impl<T> RawRecursiveDirectory<T> {
    /// Returns an iterator over the ids of the assets in the directory.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = &SharedString> {
        self.ids.iter()
    }
}

impl<T> RawRecursiveDirectory<T>
where
    T: Storable,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This fonction does not do any I/O and assets that previously failed to
    /// load are ignored.
    #[inline]
    pub fn iter_cached<'h, 'a: 'h>(
        &'h self,
        cache: &'a AssetCache,
    ) -> impl Iterator<Item = &'a Handle<T>> + 'h {
        self.ids().filter_map(move |id| cache.get(id))
    }
}

impl<T> RawRecursiveDirectory<T>
where
    T: Asset,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This function will happily try to load all assets, even if an error
    /// occured the last time it was tried.
    #[inline]
    pub fn iter<'h, 'a: 'h>(
        &'h self,
        cache: &'a AssetCache,
    ) -> impl ExactSizeIterator<Item = Result<&'a Handle<T>, Error>> + 'h {
        self.ids().map(move |id| cache.load(id))
    }
}

impl<T> fmt::Debug for RawRecursiveDirectory<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawRecursiveDirectory")
            .field("ids", &self.ids)
            .finish()
    }
}

/// Stores ids in a directory containing assets of type `T`
pub struct Directory<T> {
    ids: Vec<SharedString>,
    _marker: PhantomData<T>,
}

impl<T> Asset for Directory<T>
where
    T: DirLoadable + Asset,
{
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
        let raw = cache.load::<RawDirectory<T>>(id)?;

        let ids = &raw.read().ids;

        cache.no_record(|| {
            for id in ids {
                let _ = cache.load::<T>(id);
            }
        });

        Ok(Directory {
            ids: ids.clone(),
            _marker: PhantomData,
        })
    }

    const HOT_RELOADED: bool = true;
}

impl<T> Directory<T> {
    /// Returns an iterator over the ids of the assets in the directory.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = &SharedString> {
        self.ids.iter()
    }
}

impl<T> Directory<T>
where
    T: Storable,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This fonction does not do any I/O and assets that previously failed to
    /// load are ignored.
    #[inline]
    pub fn iter_cached<'h, 'a: 'h>(
        &'h self,
        cache: &'a AssetCache,
    ) -> impl Iterator<Item = &'a Handle<T>> + 'h {
        self.ids().filter_map(move |id| cache.get(id))
    }
}

impl<T> Directory<T>
where
    T: Asset,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This function will happily try to load all assets, even if an error
    /// occured the last time it was tried.
    #[inline]
    pub fn iter<'h, 'a: 'h>(
        &'h self,
        cache: &'a AssetCache,
    ) -> impl ExactSizeIterator<Item = Result<&'a Handle<T>, Error>> + 'h {
        self.ids().map(move |id| cache.load(id))
    }
}

impl<T> fmt::Debug for Directory<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Directory").field("ids", &self.ids).finish()
    }
}

/// Stores ids in a recursive directory containing assets of type `T`
pub struct RecursiveDirectory<T> {
    ids: Vec<SharedString>,
    _marker: PhantomData<T>,
}

impl<T> Asset for RecursiveDirectory<T>
where
    T: DirLoadable + Asset,
{
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
        let raw = cache.load::<RawRecursiveDirectory<T>>(id)?;

        let ids = &raw.read().ids;

        cache.no_record(|| {
            for id in ids {
                let _ = cache.load::<T>(id);
            }
        });

        Ok(RecursiveDirectory {
            ids: ids.clone(),
            _marker: PhantomData,
        })
    }

    const HOT_RELOADED: bool = true;
}

impl<T> RecursiveDirectory<T> {
    /// Returns an iterator over the ids of the assets in the directory.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = &SharedString> {
        self.ids.iter()
    }
}

impl<T> RecursiveDirectory<T>
where
    T: Storable,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This fonction does not do any I/O and assets that previously failed to
    /// load are ignored.
    #[inline]
    pub fn iter_cached<'h, 'a: 'h>(
        &'h self,
        cache: &'a AssetCache,
    ) -> impl Iterator<Item = &'a Handle<T>> + 'h {
        self.ids().filter_map(move |id| cache.get(id))
    }
}

impl<T> RecursiveDirectory<T>
where
    T: Asset,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This function will happily try to load all assets, even if an error
    /// occured the last time it was tried.
    #[inline]
    pub fn iter<'h, 'a: 'h>(
        &'h self,
        cache: &'a AssetCache,
    ) -> impl ExactSizeIterator<Item = Result<&'a Handle<T>, Error>> + 'h {
        self.ids().map(move |id| cache.load(id))
    }
}

impl<T> fmt::Debug for RecursiveDirectory<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecursiveDirectory")
            .field("ids", &self.ids)
            .finish()
    }
}
