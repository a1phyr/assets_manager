use crate::{
    Asset,
    AssetCache,
    Error,
    Handle,
    source::{DirEntry, Source},
    utils::{RwLock, RwLockReadGuard},
};

use std::{
    iter::FusedIterator,
    io,
    fmt,
    marker::PhantomData,
    sync::Arc
};

struct StringList {
    list: RwLock<Vec<Arc<str>>>,
}

impl From<Vec<Arc<str>>> for StringList {
    #[inline]
    fn from(vec: Vec<Arc<str>>) -> Self {
        Self {
            list: RwLock::new(vec),
        }
    }
}

impl<'a> IntoIterator for &'a StringList {
    type Item = &'a str;
    type IntoIter = StringIter<'a>;

    #[inline]
    fn into_iter(self) -> StringIter<'a> {
        let guard = self.list.read();
        let current = guard.as_ptr();
        let end = unsafe { current.add(guard.len()) };

        StringIter {
            current,
            end,

            _guard: guard,
        }
    }
}

struct StringIter<'a> {
    current: *const Arc<str>,
    end: *const Arc<str>,

    _guard: RwLockReadGuard<'a, Vec<Arc<str>>>,
}

impl<'a> Iterator for StringIter<'a> {
    type Item = &'a str;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current == self.end {
            None
        } else {
            unsafe {
                let string = &*self.current;
                self.current = self.current.offset(1);
                Some(string)
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl ExactSizeIterator for StringIter<'_> {
    #[inline]
    fn len(&self) -> usize {
        let diff = (self.end as usize) - (self.current as usize);
        diff / std::mem::size_of::<Arc<str>>()
    }
}

impl FusedIterator for StringIter<'_> {}

pub(crate) struct CachedDir {
    assets: Box<StringList>,
}

impl CachedDir {
    pub fn load<A: Asset, S: Source>(cache: &AssetCache<S>, dir_id: &str) -> io::Result<Self> {
        let mut ids = Vec::new();

        cache.source().read_dir(dir_id, &mut |entry| {
            if let DirEntry::File(id, ext) = entry {
                if A::EXTENSIONS.contains(&ext) {
                    let _ = cache.load::<A>(&id);
                    ids.push(id.into());
                }
            }
        })?;

        Ok(Self {
            assets: Box::new(ids.into()),
        })
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn contains(&self, id: &str) -> bool {
        self.assets.into_iter().any(|s| s == id)
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn add(&self, id: &Arc<str>) -> bool {
        let insert = !self.contains(&id);

        if insert {
            let mut list = self.assets.list.write();
            list.push(id.clone());
        }

        insert
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn remove(&self, id: &str) -> bool {
        let mut list = self.assets.list.write();

        if let Some(pos) = list.iter().position(|s| s.as_ref() == id) {
            list.remove(pos);
            true
        } else {
            false
        }
    }

    #[inline]
    pub unsafe fn read<'a, A, S>(&self, cache: &'a AssetCache<S>) -> DirReader<'a, A, S> {
        DirReader {
            cache,
            assets: &*(&*self.assets as *const StringList),
            _marker: PhantomData,
        }
    }
}

impl fmt::Debug for CachedDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&*self.assets.list.read()).finish()
    }
}

/// A reference to all assets in a directory.
///
/// This type provides methods to iterates over theses assets.
///
/// When [hot-reloading] is used, added/removed files will be added/removed from
/// this structure.
///
/// This structure can be obtained by calling [`AssetCache::load_dir`].
///
/// [hot-reloading]: `AssetCache::hot_reload`
pub struct DirReader<'a, A, S> {
    cache: &'a AssetCache<S>,
    assets: &'a StringList,
    _marker: PhantomData<&'a A>,
}

impl<A, S> Clone for DirReader<'_, A, S> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<A, S> Copy for DirReader<'_, A, S> {}

impl<'a, A: Asset, S> DirReader<'a, A, S> {
    /// An iterator over successfully loaded assets in a directory.
    ///
    /// This iterator yields each asset that was successfully loaded. It is
    /// garantied to perform no I/O. This is the method you want to use most of
    /// the time.
    ///
    /// Note that if an asset is [removed from the cache], it won't be returned
    /// by this iterator until it is cached again.
    ///
    /// [removed from the cache]: `AssetCache::remove`
    #[inline]
    pub fn iter(&self) -> ReadDir<'a, A, S> {
        ReadDir {
            cache: self.cache,
            iter: self.assets.into_iter(),
            _marker: PhantomData,
        }
    }

    /// An iterator over all assets in a directory.
    ///
    /// This iterator yields the id asset of each asset in a directory, with the
    /// result of its last loading from the cache. It will happily try to reload
    /// any asset that is not in the cache (e.g. that previously failed to load
    /// or was removed).
    #[inline]
    pub fn iter_all(&self) -> ReadAllDir<'a, A, S> {
        ReadAllDir {
            cache: self.cache,
            iter: self.assets.into_iter(),
            _marker: PhantomData,
        }
    }
}

impl<'a, A, S> IntoIterator for &DirReader<'a, A, S>
where
    A: Asset,
    S: Source,
{
    type Item = Handle<'a, A>;
    type IntoIter = ReadDir<'a, A, S>;

    /// Equivalent to [`iter`](#method.iter).
    #[inline]
    fn into_iter(self) -> ReadDir<'a, A, S> {
        self.iter()
    }
}

/// An iterator over successfully loaded assets in a directory.
///
/// This iterator yields each asset that was successfully loaded.
///
/// It can be obtained by calling [`DirReader::iter`].
pub struct ReadDir<'a, A, S> {
    cache: &'a AssetCache<S>,
    iter: StringIter<'a>,
    _marker: PhantomData<&'a A>,
}

impl<'a, A, S> Iterator for ReadDir<'a, A, S>
where
    A: Asset,
    S: Source,
{
    type Item = Handle<'a, A>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let id = self.iter.next()?;

            if let asset @ Some(_) = self.cache.load_cached(id) {
                break asset;
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.iter.size_hint().1)
    }
}

impl<A, S> FusedIterator for ReadDir<'_, A, S>
where
    A: Asset,
    S: Source,
{}

/// An iterator over all assets in a directory.
///
/// This iterator yields the id asset of each asset in a directory, with the
/// result of its loading from the cache.
///
/// It can be obtained by calling [`DirReader::iter_all`].
pub struct ReadAllDir<'a, A, S> {
    cache: &'a AssetCache<S>,
    iter: StringIter<'a>,
    _marker: PhantomData<&'a A>,
}

impl<'a, A, S> Iterator for ReadAllDir<'a, A, S>
where
    A: Asset,
    S: Source,
{
    type Item = (&'a str, Result<Handle<'a, A>, Error>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.iter.next()?;
        Some((id, self.cache.load(id)))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<A, S> ExactSizeIterator for ReadAllDir<'_, A, S>
where
    A: Asset,
    S: Source,
{
    #[inline]
    fn len(&self) -> usize {
        self.iter.len()
    }
}

impl<A, S> FusedIterator for ReadAllDir<'_, A, S>
where
    A: Asset,
    S: Source,
{}

impl<A, S> fmt::Debug for DirReader<'_, A, S>
where
    A: fmt::Debug + Asset,
    S: Source,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<A, S> fmt::Debug for ReadDir<'_, A, S>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadDir").finish()
    }
}

impl<A, S> fmt::Debug for ReadAllDir<'_, A, S>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadAllDir").finish()
    }
}
