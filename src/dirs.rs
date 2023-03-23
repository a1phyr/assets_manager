use crate::{
    source::{DirEntry, Source},
    AnyCache, Asset, BoxedError, Compound, Error, Handle, SharedString,
};

use std::{fmt, io, marker::PhantomData};

#[cfg(doc)]
use crate::AssetCache;

/// Assets that are loadable from directories
///
/// Types that implement this trait can be used with [`AssetCache::load_dir`] to
/// load all available assets in a directory (eventually recursively).
///
/// This trait is automatically implemented for all types that implement
/// [`Asset`], and you can implement it to extend your own `Compound`s.
///
/// # Exemple implementation
///
/// Imagine you have several playlists with a JSON manifest to specify the ids
/// of the musics to include.
///
/// ```no_run
/// # cfg_if::cfg_if! { if #[cfg(all(feature = "json", feature = "flac"))] {
/// use assets_manager::{
///     Compound, BoxedError, AnyCache, SharedString,
///     asset::{DirLoadable, Json, Flac},
///     source::{DirEntry, Source},
/// };
///
/// /// A simple playlist, a mere ordered list of musics
/// struct Playlist {
///     sounds: Vec<Flac>
/// }
///
/// // Specify how to load a playlist
/// impl Compound for Playlist {
///     fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
///         // Read the manifest (a list of ids)
///         let manifest = cache.load::<Json<Vec<String>>>(id)?.read();
///
///         // Load each sound
///         let sounds = manifest.0.iter()
///             .map(|id| Ok(cache.load::<Flac>(id)?.cloned()))
///             .collect::<Result<_, BoxedError>>()?;
///
///         Ok(Playlist { sounds })
///     }
/// }
///
/// // Specify how to get ids of playlists in a directory
/// impl DirLoadable for Playlist {
///     fn select_ids(cache: AnyCache, id: &SharedString) -> std::io::Result<Vec<SharedString>> {
///         let mut ids = Vec::new();
///
///         // Select all files with "json" extension (manifest files)
///         cache.raw_source().read_dir(id, &mut |entry| {
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
pub trait DirLoadable: Send + Sync + 'static {
    /// Returns the ids of the assets contained in the directory given by `id`.
    ///
    /// Note that the order of the returned ids is not kept, and that redundant
    /// ids are removed.
    fn select_ids(cache: AnyCache, id: &SharedString) -> io::Result<Vec<SharedString>>;

    /// Executes the given closure for each id of a child directory of the given
    /// directory. The default implementation reads the cache's source.
    #[inline]
    fn sub_directories(
        cache: AnyCache,
        id: &SharedString,
        mut f: impl FnMut(&str),
    ) -> io::Result<()> {
        cache.raw_source().read_dir(id, &mut |entry| {
            if let DirEntry::Directory(id) = entry {
                f(id);
            }
        })
    }
}

impl<A> DirLoadable for A
where
    A: Asset,
{
    #[inline]
    fn select_ids(cache: AnyCache, id: &SharedString) -> io::Result<Vec<SharedString>> {
        fn inner(cache: AnyCache, id: &str, extensions: &[&str]) -> io::Result<Vec<SharedString>> {
            let mut ids = Vec::new();

            // Select all files with an extension valid for type `A`
            cache.raw_source().read_dir(id, &mut |entry| {
                if let DirEntry::File(id, ext) = entry {
                    if extensions.contains(&ext) {
                        ids.push(id.into());
                    }
                }
            })?;

            Ok(ids)
        }

        inner(cache, id, A::EXTENSIONS)
    }
}

impl<A> DirLoadable for std::sync::Arc<A>
where
    A: DirLoadable,
{
    #[inline]
    fn select_ids(cache: AnyCache, id: &SharedString) -> io::Result<Vec<SharedString>> {
        A::select_ids(cache, id)
    }
}

/// Stores ids in a directory containing assets of type `A`
pub(crate) struct CachedDir<A> {
    ids: Vec<SharedString>,
    _marker: PhantomData<A>,
}

impl<A> Compound for CachedDir<A>
where
    A: DirLoadable,
{
    fn load(cache: crate::AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        let mut ids = cache
            .no_record(|| A::select_ids(cache, id))
            .map_err(|err| Error::from_io(id.clone(), err))?;

        // Remove duplicated entries
        ids.sort_unstable();
        ids.dedup();

        Ok(CachedDir {
            ids,
            _marker: PhantomData,
        })
    }

    const HOT_RELOADED: bool = false;
}

impl<A: DirLoadable> crate::asset::NotHotReloaded for CachedDir<A> {}

impl<A> fmt::Debug for CachedDir<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.ids.fmt(f)
    }
}

/// Stores ids in a recursive directory containing assets of type `A`
pub(crate) struct CachedRecDir<A> {
    ids: Vec<SharedString>,
    _marker: PhantomData<A>,
}

impl<A> Compound for CachedRecDir<A>
where
    A: DirLoadable,
{
    fn load(cache: crate::AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        // Load the current directory
        let this = cache.load::<CachedDir<A>>(id)?;
        let mut ids = this.get().ids.clone();

        // Recursively load child directories
        A::sub_directories(cache, id, |id| {
            if let Ok(child) = cache.load::<CachedRecDir<A>>(id) {
                ids.extend_from_slice(&child.get().ids);
            }
        })
        .map_err(|err| Error::from_io(id.clone(), err))?;

        Ok(CachedRecDir {
            ids,
            _marker: PhantomData,
        })
    }

    const HOT_RELOADED: bool = false;
}

impl<A: DirLoadable> crate::asset::NotHotReloaded for CachedRecDir<A> {}

impl<A> fmt::Debug for CachedRecDir<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.ids.fmt(f)
    }
}

enum DirHandleInner<'a, A> {
    Simple(Handle<'a, CachedDir<A>>),
    Recursive(Handle<'a, CachedRecDir<A>>),
}

impl<A> Clone for DirHandleInner<'_, A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for DirHandleInner<'_, A> {}

impl<'a, A> DirHandleInner<'a, A>
where
    A: DirLoadable,
{
    #[inline]
    fn id(self) -> &'a SharedString {
        match self {
            Self::Simple(handle) => handle.id(),
            Self::Recursive(handle) => handle.id(),
        }
    }

    #[inline]
    fn ids(self) -> &'a [SharedString] {
        match self {
            Self::Simple(handle) => &handle.get().ids,
            Self::Recursive(handle) => &handle.get().ids,
        }
    }
}

/// A handle on a asset directory.
///
/// This type provides methods to access assets within a directory.
pub struct DirHandle<'a, A> {
    inner: DirHandleInner<'a, A>,
}

impl<'a, A> DirHandle<'a, A>
where
    A: DirLoadable,
{
    #[inline]
    pub(crate) fn new(handle: Handle<'a, CachedDir<A>>) -> Self {
        let inner = DirHandleInner::Simple(handle);
        DirHandle { inner }
    }

    #[inline]
    pub(crate) fn new_rec(handle: Handle<'a, CachedRecDir<A>>) -> Self {
        let inner = DirHandleInner::Recursive(handle);
        DirHandle { inner }
    }

    /// The id of the directory handle.
    #[inline]
    pub fn id(self) -> &'a SharedString {
        self.inner.id()
    }

    /// Returns an iterator over the ids of the assets in the directory.
    #[inline]
    pub fn ids(self) -> impl ExactSizeIterator<Item = &'a SharedString> {
        self.inner.ids().iter()
    }
}

impl<'h, A> DirHandle<'h, A>
where
    A: DirLoadable + crate::Storable,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This fonction does not do any I/O and assets that previously failed to
    /// load are ignored.
    #[inline]
    pub fn iter_cached<'a: 'h>(
        self,
        cache: AnyCache<'a>,
    ) -> impl Iterator<Item = Handle<'a, A>> + 'h {
        self.ids().filter_map(move |id| cache.get_cached(id))
    }
}

impl<'h, A> DirHandle<'h, A>
where
    A: DirLoadable + Compound,
{
    /// Returns an iterator over the assets in the directory.
    ///
    /// This function will happily try to load all assets, even if an error
    /// occured the last time it was tried.
    #[inline]
    pub fn iter<'a: 'h>(
        self,
        cache: AnyCache<'a>,
    ) -> impl ExactSizeIterator<Item = Result<Handle<'a, A>, Error>> + 'h {
        self.ids().map(move |id| cache.load(id))
    }
}

impl<A> Clone for DirHandle<'_, A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for DirHandle<'_, A> {}

impl<A> fmt::Debug for DirHandle<'_, A>
where
    A: DirLoadable,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirHandle")
            .field("ids", &self.inner.ids())
            .finish()
    }
}
