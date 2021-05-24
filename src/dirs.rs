use crate::{
    Asset,
    AssetCache,
    Compound,
    Error,
    Handle,
    source::{DirEntry, Source},
    utils::SharedString,
};

use std::{
    io,
    fmt,
    marker::PhantomData,
};

/// Helper type to implement [`DirLoadable`].
///
/// This type provides a way to execute a function for each file in a directory.
pub struct Directory<'a> {
    source: &'a dyn Source,
    id: &'a str,
}

impl<'a> Directory<'a> {
    /// Returns the id of the directory.
    #[inline]
    pub fn id(&self) -> &'a str {
        self.id
    }

    /// Iterates over all files in the directory.
    ///
    /// The given closure is executed for each file in the directory, one none
    /// if the function returns `Err`.
    #[inline]
    pub fn for_each_file(&self, mut f: impl FnMut(&str, &str)) -> io::Result<()> {
        self.source.read_dir(self.id, &mut |entry| match entry {
            DirEntry::File(id, ext) => f(id, ext),
            DirEntry::Directory(_) => (),
        })
    }
}

impl fmt::Debug for Directory<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Directory")
            .field("id", &self.id)
            .finish()
    }
}

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
///     Compound, Error, AssetCache,
///     asset::{DirLoadable, Json, Flac},
///     source::Source,
///     utils::{Directory, SharedString},
/// };
///
/// /// A simple playlist, a mere ordered list of musics
/// struct Playlist {
///     sounds: Vec<Flac>
/// }
///
/// // Specify how to load a playlist
/// impl Compound for Playlist {
///     fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
///         // Read the manifest (a list of ids)
///         let manifest = cache.load::<Json<Vec<String>>>(id)?.read();
///
///         // Load each sound
///         let sounds = manifest.0.iter()
///             .map(|id| Ok(cache.load::<Flac>(id)?.cloned()))
///             .collect::<Result<_, Error>>()?;
///
///         Ok(Playlist { sounds })
///     }
/// }
///
/// // Specify how to get ids of playlists in a directory
/// impl DirLoadable for Playlist {
///     fn select_ids(dir: Directory) -> std::io::Result<Vec<SharedString>> {
///         let mut ids = Vec::new();
///
///         // Select all files with the "json" extensions (manifest files)
///         dir.for_each_file(|id, ext| {
///             if ext == "json" {
///                 ids.push(id.into());
///             }
///         })?;
///
///         Ok(ids)
///     }
/// }
/// # }}
/// ```
pub trait DirLoadable: Compound {
    /// Returns the ids of the assets contained in the directory.
    ///
    /// The list of files and the id of the directory can be retreived through
    /// the `files` parameter.
    ///
    /// Note that the order of the returned ids is not kept, and that redundant
    /// ids are removed.
    fn select_ids(dir: Directory) -> io::Result<Vec<SharedString>>;
}

impl<A> DirLoadable for A
where
    A: Asset,
{
    #[inline]
    fn select_ids(dir: Directory) -> io::Result<Vec<SharedString>> {
        fn inner(dir: Directory, extensions: &[&str]) -> io::Result<Vec<SharedString>> {
            let mut ids = Vec::new();

            dir.for_each_file(|id, ext| {
                if extensions.contains(&ext) {
                    ids.push(id.into());
                }
            })?;

            Ok(ids)
        }

        inner(dir, A::EXTENSIONS)
    }
}

pub(crate) struct CachedDir<A> {
    ids: Vec<SharedString>,
    _marker: PhantomData<A>,
}

impl<A> Compound for CachedDir<A>
where
    A: DirLoadable,
{
    fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        let files = Directory { id, source: cache.source() };
        let mut ids = A::select_ids(files)?;

        // Remove deduplicated entries
        ids.sort_unstable();
        ids.dedup();

        // Pre-load inner assets
        cache.no_record(|| {
            for id in &ids {
                let _ = cache.load::<A>(id);
            }
        });

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

pub(crate) struct CachedRecDir<A> {
    ids: Vec<SharedString>,
    _marker: PhantomData<A>,
}

impl<A> Compound for CachedRecDir<A>
where
    A: DirLoadable,
{
    fn load<S: Source>(cache: &AssetCache<S>, id: &str) -> Result<Self, Error> {
        // Load the current directory
        let this = cache.load::<CachedDir<A>>(id)?;
        let mut ids = this.get().ids.clone();

        // Recursively load child directories
        cache.source().read_dir(id, &mut |entry| {
            if let DirEntry::Directory(id) = entry {
                if let Ok(child) = cache.load::<CachedRecDir<A>>(id) {
                    ids.extend_from_slice(&child.get().ids);
                }
            }
        })?;

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
    fn id(self) -> &'a str {
        match self {
            Self::Simple(handle) => &handle.id(),
            Self::Recursive(handle) => &handle.id(),
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
/// This type provides methods to access assets within th directory.
pub struct DirHandle<'a, A, S> {
    inner: DirHandleInner<'a, A>,
    cache: &'a AssetCache<S>,
}

impl<'a, A, S> DirHandle<'a, A, S>
where
    A: DirLoadable,
    S: Source,
{
    #[inline]
    pub(crate) fn new(handle: Handle<'a, CachedDir<A>>, cache: &'a AssetCache<S>) -> Self {
        let inner = DirHandleInner::Simple(handle);
        DirHandle { inner, cache }
    }

    #[inline]
    pub(crate) fn new_rec(handle: Handle<'a, CachedRecDir<A>>, cache: &'a AssetCache<S>) -> Self {
        let inner = DirHandleInner::Recursive(handle);
        DirHandle { inner, cache }
    }

    /// The id of the directory handle.
    #[inline]
    pub fn id(self) -> &'a str {
        self.inner.id()
    }

    /// Returns an iterator over the ids of the assets in the directory.
    #[inline]
    pub fn ids(self) -> impl ExactSizeIterator<Item=&'a str> {
        self.inner.ids().iter().map(|id| &**id)
    }

    /// Returns an iterator over the assets in the directory.
    ///
    /// This fonction does not do any I/O and assets that previously failed to
    /// load are ignored.
    #[inline]
    pub fn iter(self) -> impl Iterator<Item=Handle<'a, A>> {
        self.inner.ids().iter().filter_map(move |id| self.cache.get_cached(&**id))
    }

    /// Returns an iterator over the assets in the directory.
    ///
    /// Unlike `Self::iter`, this function will happily try to load all assets, even
    /// if an error occured the last time it was tried.
    ///
    /// The iterator is a list of tuples (id, result of the `load` operation).
    #[inline]
    pub fn iter_all(self) -> impl ExactSizeIterator<Item=(&'a str, Result<Handle<'a, A>, Error>)> {
        self.inner.ids().iter().map(move |id| (&**id, self.cache.load(&**id)))
    }
}

impl<A, S> Clone for DirHandle<'_, A, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A, S> Copy for DirHandle<'_, A, S> {}

impl<A, S> fmt::Debug for DirHandle<'_, A, S>
where
    A: DirLoadable,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirHandle").field("ids", &self.inner.ids()).finish()
    }
}
