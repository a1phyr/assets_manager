//! Definitions of cache entries

use std::{
    any::Any,
    fmt,
    hash,
    ops::Deref,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::utils::{RwLock, RwLockReadGuard};

struct Inner<T> {
    lock: RwLock<T>,
    reload: AtomicUsize,
}

impl<T> Inner<T> {
    #[inline]
    fn new(value: T) -> Self {
        Self {
            lock: RwLock::new(value),
            reload: AtomicUsize::new(0),
        }
    }

    #[inline]
    fn write(&self, value: T) {
        let mut data = self.lock.write();
        *data = value;
        self.reload.fetch_add(1, Ordering::Release);
    }
}


/// An entry in the cache
///
/// # Safety
///
/// - Methods that are generic over `T` can only be called with the same `T` used
/// to create them.
/// - When an `AssetRef<'a, T>` is returned, you have to ensure that `self`
/// outlives it. The `CacheEntry` can be moved but cannot be dropped.
///
/// [`ContreteCacheEntry`]: struct.ContreteCacheEntry.html
pub(crate) struct CacheEntry(Box<dyn Any + Send + Sync>);

impl<'a> CacheEntry {
    /// Creates a new `CacheEntry` containing an asset of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    #[inline]
    pub fn new<T: Send + Sync + 'static>(asset: T) -> Self {
        CacheEntry(Box::new(Inner::new(asset)))
    }

    /// Returns a reference to the underlying lock.
    ///
    /// # Safety
    ///
    /// See type-level documentation.
    #[inline]
    pub unsafe fn get_ref<T: Send + Sync + 'static>(&self) -> AssetRef<'a, T> {
        debug_assert!(self.0.is::<Inner<T>>());

        let data = {
            let ptr = &*self.0 as *const dyn Any as *const Inner<T>;
            &*ptr
        };

        AssetRef::new(data)
    }

    /// Write a value and a get reference to the underlying lock
    ///
    /// # Safety
    ///
    /// See type-level documentation.
    pub unsafe fn write<T: Send + Sync + 'static>(&self, asset: T) -> AssetRef<'a, T> {
        let lock = self.get_ref();
        lock.data.write(asset);
        lock
    }

    /// Consumes the `CacheEntry` and returns its inner value.
    ///
    /// # Safety
    ///
    /// See type-level documentation.
    #[inline]
    pub unsafe fn into_inner<T: Send + Sync + 'static>(self) -> T {
        debug_assert!(self.0.is::<Inner<T>>());

        let inner = Box::from_raw(Box::into_raw(self.0) as *mut Inner<T>);
        inner.lock.into_inner()
    }
}

impl fmt::Debug for CacheEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("CacheEntry")
    }
}


/// A lock on an asset.
///
/// The type parameter `A` represents type of the locked asset.
///
/// This structure wraps a RwLock, so assets can be written to be reloaded. As
/// such, any number of read guard can exist at the same time, but none can
/// exist while reloading an asset.
///
/// This is the structure you want to use to store a reference to an asset.
/// However, data shared between threads is usually required to be `'static`,
/// which is usually false for this structure. The preferred way to share assets
/// is to share the `AssetCache` and to load assets from it whenever you need
/// it: it is a very cheap operation. You can also create a `&'static AssetCache`
/// (for example with `lazy_static` crate or by [leaking a `Box`]), but doing
/// this prevents from removing assets from the cache. Another solution is to
/// use crates that allow threads with non-static data (such as
/// `crossbeam-utils::scope`).
///
/// [leaking a `Box`]: https://doc.rust-lang.org/std/boxed/struct.Box.html#method.leak
pub struct AssetRef<'a, A> {
    data: &'a Inner<A>,
    last_reload: usize,
}

impl<'a, A> AssetRef<'a, A> {
    #[inline]
    fn new(inner: &'a Inner<A>) -> Self {
        Self {
            data: inner,
            last_reload: inner.reload.load(Ordering::Acquire),
        }
    }

    /// Locks the pointed asset for reading.
    ///
    /// Returns a RAII guard which will release the lock once dropped.
    #[inline]
    pub fn read(&self) -> AssetGuard<'a, A> {
        AssetGuard {
            guard: self.data.lock.read(),
        }
    }

    /// Returns `true` if the asset has been reloaded since last call to this
    /// method with this `AssetRef`.
    ///
    /// # Example
    ///
    /// ```
    /// use assets_manager::{Asset, AssetCache};
    /// # use assets_manager::loader::{LoadFrom, ParseLoader};
    ///
    /// struct Example;
    /// # impl From<i32> for Example {
    /// #     fn from(n: i32) -> Self { Self }
    /// # }
    /// impl Asset for Example {
    ///     /* ... */
    ///     # const EXTENSION: &'static str = "x";
    ///     # type Loader = LoadFrom<i32, ParseLoader>;
    /// }
    ///
    /// let cache = AssetCache::new("assets")?;
    ///
    /// let mut ref1 = cache.load::<Example>("example.reload")?;
    /// let mut ref2 = cache.load::<Example>("example.reload")?;
    ///
    /// assert!(!ref1.reloaded());
    ///
    /// cache.force_reload::<Example>("example.reload")?;
    ///
    /// assert!(ref1.reloaded());
    /// assert!(!ref1.reloaded());
    ///
    /// assert!(ref2.reloaded());
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn reloaded(&mut self) -> bool {
        let last_reload = self.data.reload.load(Ordering::Acquire);

        if last_reload > self.last_reload {
            self.last_reload = last_reload;
            true
        } else {
            false
        }
    }

    /// Checks if the two assets refer to the same cache entry.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.data, other.data)
    }
}

impl<A> AssetRef<'_, A>
where
    A: Clone
{
    /// Returns a clone of the inner asset.
    #[inline]
    pub fn cloned(self) -> A {
        self.data.lock.read().clone()
    }
}

impl<A> Clone for AssetRef<'_, A> {
    fn clone(&self) -> Self {
        Self {
            data: self.data,
            last_reload: self.last_reload,
        }
    }
}

impl<A> Copy for AssetRef<'_, A> {}

impl<T, U> PartialEq<AssetRef<'_, U>> for AssetRef<'_, T>
where
    T: PartialEq<U>,
{
    fn eq(&self, other: &AssetRef<U>) -> bool {
        self.data.lock.read().eq(&other.data.lock.read())
    }
}

impl<A> Eq for AssetRef<'_, A> where A: Eq {}

impl<A> hash::Hash for AssetRef<'_, A>
where
    A: hash::Hash,
{
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.data.lock.read().hash(state);
    }
}

impl<A> fmt::Debug for AssetRef<'_, A>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetRef").field("data", &*self.data.lock.read()).finish()
    }
}


/// RAII guard used to keep a read lock on an asset and release it when dropped.
///
/// This type is a smart pointer to type `A`.
///
/// It can be obtained by calling [`AssetRef::read`].
///
/// [`AssetRef::read`]: struct.AssetRef.html#method.read
pub struct AssetGuard<'a, A> {
    guard: RwLockReadGuard<'a, A>,
}

impl<A> Deref for AssetGuard<'_, A> {
    type Target = A;

    #[inline]
    fn deref(&self) -> &A {
        &self.guard
    }
}

impl<A> fmt::Display for AssetGuard<'_, A>
where
    A: fmt::Display,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<A> fmt::Debug for AssetGuard<'_, A>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
