//! Definitions of cache entries

use std::{
    any::Any,
    fmt,
    ops::Deref,
    sync::Arc,
};

#[cfg(feature = "hot-reloading")]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(feature = "hot-reloading")]
use crate::utils::{RwLock, RwLockReadGuard};


#[cfg(feature = "hot-reloading")]
pub struct Inner<T> {
    id: Arc<str>,
    reload: AtomicUsize,
    reload_global: AtomicBool,

    value: RwLock<T>,
}

#[cfg(feature = "hot-reloading")]
impl<T> Inner<T> {
    #[inline]
    fn new(value: T, id: Arc<str>) -> Self {
        Self {
            id,
            reload: AtomicUsize::new(0),
            reload_global: AtomicBool::new(false),

            value: RwLock::new(value),
        }
    }

    #[inline]
    pub fn write(&self, value: T) {
        let mut data = self.value.write();
        *data = value;
        self.reload.fetch_add(1, Ordering::Release);
        self.reload_global.store(true, Ordering::Release);
    }

    #[inline]
    fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

#[cfg(not(feature = "hot-reloading"))]
pub struct Inner<T> {
    id: Arc<str>,
    value: T,
}

#[cfg(not(feature = "hot-reloading"))]
impl<T> Inner<T> {
    #[inline]
    fn new(value: T, id: Arc<str>) -> Self {
        Self { id, value }
    }

    #[inline]
    fn into_inner(self) -> T {
        self.value
    }
}


/// An entry in the cache
///
/// # Safety
///
/// - Methods that are generic over `T` can only be called with the same `T` used
/// to create them.
/// - When an `Handle<'a, T>` is returned, you have to ensure that `self`
/// outlives it. The `CacheEntry` can be moved but cannot be dropped.
pub(crate) struct CacheEntry(pub Box<dyn Any + Send + Sync>);

impl<'a> CacheEntry {
    /// Creates a new `CacheEntry` containing an asset of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    #[inline]
    pub fn new<T: Send + Sync + 'static>(asset: T, id: Arc<str>) -> Self {
        CacheEntry(Box::new(Inner::new(asset, id)))
    }

    #[inline]
    pub unsafe fn inner<T: Send + Sync + 'static>(&self) -> &'a Inner<T> {
        debug_assert!(self.0.is::<Inner<T>>());

        let ptr = &*self.0 as *const dyn Any as *const Inner<T>;
        &*ptr
    }

    /// Returns a reference to the underlying lock.
    ///
    /// # Safety
    ///
    /// See type-level documentation.
    #[inline]
    pub unsafe fn get_ref<T: Send + Sync + 'static>(&self) -> Handle<'a, T> {
        let inner = self.inner::<T>();
        Handle::new(inner)
    }

    /// Write a value and a get reference to the underlying lock
    ///
    /// # Safety
    ///
    /// See type-level documentation.
    #[cfg(feature = "hot-reloading")]
    pub unsafe fn write<T: Send + Sync + 'static>(&self, asset: T) -> Handle<'a, T> {
        let inner = self.inner::<T>();
        inner.write(asset);
        Handle::new(inner)
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
        inner.into_inner()
    }
}

impl fmt::Debug for CacheEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheEntry").finish()
    }
}


/// A handle on an asset.
///
/// Such a handle can be used to get access to an asset of type `A`. It is
/// generally obtained by call `AssetCache::load` and its variants.
///
/// If feature `hot-reloading` is used, this structure wraps a RwLock, so
/// assets can be written to be reloaded. As such, any number of read guard can
/// exist at the same time, but none can exist while reloading an asset (when
/// calling `AssetCache::hot_reload`).
///
/// This is the structure you want to use to store a reference to an asset.
/// However it is generally easier to work with `'static` data. For more
/// information, see [top-level documentation](index.html#becoming-static).
pub struct Handle<'a, A> {
    data: &'a Inner<A>,

    #[cfg(feature = "hot-reloading")]
    last_reload: usize,
}

impl<'a, A> Handle<'a, A> {
    #[inline]
    fn new(inner: &'a Inner<A>) -> Self {
        Self {
            data: inner,

            #[cfg(feature = "hot-reloading")]
            last_reload: inner.reload.load(Ordering::Acquire),
        }
    }

    /// Locks the pointed asset for reading.
    ///
    /// Returns a RAII guard which will release the lock once dropped.
    #[inline]
    pub fn read(&self) -> AssetGuard<'a, A> {
        AssetGuard {
            #[cfg(feature = "hot-reloading")]
            asset: self.data.value.read(),

            #[cfg(not(feature = "hot-reloading"))]
            asset: &self.data.value,
        }
    }

    /// Returns the id of the asset
    #[inline]
    pub fn id(&self) -> &'a str {
        &self.data.id
    }

    /// Returns `true` if the asset has been reloaded since last call to this
    /// method with the same handle.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # cfg_if::cfg_if! { if #[cfg(feature = "hot-reloading")] {
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
    /// let mut asset = cache.load::<Example>("example.reload")?;
    ///
    /// // The handle has just been created, so `reloaded` returns false
    /// assert!(!asset.reloaded());
    ///
    /// loop {
    ///     cache.hot_reload();
    ///
    ///     if asset.reloaded() {
    ///         println!("The asset was reloaded !")
    ///     }
    ///
    ///     // Calling `reloaded` once more returns false: the asset has not
    ///     // been reloaded since last call to `reloaded`
    ///     assert!(!asset.reloaded());
    /// }
    ///
    /// # }}
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    #[inline]
    pub fn reloaded(&mut self) -> bool {
        #[cfg(feature = "hot-reloading")]
        {
            let last_reload = self.data.reload.load(Ordering::Acquire);

            if last_reload > self.last_reload {
                self.last_reload = last_reload;
                true
            } else {
                false
            }
        }

        #[cfg(not(feature = "hot-reloading"))]
        { false }
    }

    /// Returns `true` if the asset has been reloaded since last call to this
    /// method with **any** handle on this asset.
    ///
    /// Note that this method and [`reloaded`] are totally independant, and
    /// the result of the two functions do not depend on whether the other was
    /// called
    ///
    /// [`reloaded`]: Self::reloaded
    #[inline]
    pub fn reloaded_global(&self) -> bool {
        #[cfg(feature = "hot-reloading")]
        { self.data.reload_global.swap(false, Ordering::Acquire) }

        #[cfg(not(feature = "hot-reloading"))]
        { false }
    }

    /// Checks if the two handles refer to the same asset.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.data, other.data)
    }
}

impl<A> Handle<'_, A>
where
    A: Copy
{
    /// Returns a copy of the inner asset.
    ///
    /// This is functionnally equivalent to `cloned`, but it ensures that no
    /// expensive operation is used (eg if a type is refactored).
    #[inline]
    pub fn copied(self) -> A {
        *self.read()
    }
}

impl<A> Handle<'_, A>
where
    A: Clone
{
    /// Returns a clone of the inner asset.
    #[inline]
    pub fn cloned(self) -> A {
        self.read().clone()
    }
}

impl<A> Clone for Handle<'_, A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for Handle<'_, A> {}

impl<T, U> PartialEq<Handle<'_, U>> for Handle<'_, T>
where
    T: PartialEq<U>,
{
    fn eq(&self, other: &Handle<U>) -> bool {
        self.read().eq(&other.read())
    }
}

impl<A> Eq for Handle<'_, A> where A: Eq {}

impl<A> fmt::Debug for Handle<'_, A>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle").field("value", &*self.read()).finish()
    }
}


/// RAII guard used to keep a read lock on an asset and release it when dropped.
///
/// This type is a smart pointer to type `A`.
///
/// It can be obtained by calling [`Handle::read`].
pub struct AssetGuard<'a, A> {
    #[cfg(feature = "hot-reloading")]
    asset: RwLockReadGuard<'a, A>,

    #[cfg(not(feature = "hot-reloading"))]
    asset: &'a A,
}

impl<A> Deref for AssetGuard<'_, A> {
    type Target = A;

    #[inline]
    fn deref(&self) -> &A {
        &self.asset
    }
}

impl<A, U> AsRef<U> for AssetGuard<'_, A>
where
    A: AsRef<U>
{
    #[inline]
    fn as_ref(&self) -> &U {
        self.asset.as_ref()
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
