//! Definitions of cache entries

use std::{
    any::{Any, type_name},
    fmt,
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use crate::{
    asset::NotHotReloaded,
    utils::{RwLock, RwLockReadGuard},
};


/// The representation of an asset whose value cannot change.
pub(crate) struct StaticInner<T> {
    id: Arc<str>,
    value: T,
}

impl<T> StaticInner<T> {
    #[inline]
    fn new(value: T, id: Arc<str>) -> Self {
        Self { id, value }
    }
}

/// The representation of an asset whose value can be updated (eg through
/// hot-reloading).
pub(crate) struct DynamicInner<T> {
    id: Arc<str>,
    value: RwLock<T>,
    reload_global: AtomicBool,
    reload: AtomicUsize,
}

#[cfg(feature = "hot-reloading")]
impl<T> DynamicInner<T> {
    #[inline]
    fn new(value: T, id: Arc<str>) -> Self {
        Self {
            id,
            value: RwLock::new(value),
            reload: AtomicUsize::new(0),
            reload_global: AtomicBool::new(false),
        }
    }

    pub fn write(&self, value: T) {
        let mut data = self.value.write();
        *data = value;
        self.reload.fetch_add(1, Ordering::Release);
        self.reload_global.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CacheEntryInner<'a>(&'a (dyn Any + Send + Sync));

impl<'a> CacheEntryInner<'a> {
    #[inline]
    pub unsafe fn extend_lifetime<'b>(self) -> CacheEntryInner<'b> {
        let inner = &*(self.0 as *const (dyn Any + Send + Sync));
        CacheEntryInner(inner)
    }

    #[inline]
    pub fn handle<T: 'static>(self) -> Handle<'a, T> {
        Handle::new(self)
    }
}

/// An entry in the cache.
pub(crate) struct CacheEntry(pub Box<dyn Any + Send + Sync>);

impl CacheEntry {
    /// Creates a new `CacheEntry` containing an asset of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    #[inline]
    pub fn new<T: Send + Sync + 'static>(asset: T, id: Arc<str>, _reloadable: bool) -> Self {
        #[cfg(not(feature = "hot-reloading"))]
        let inner = Box::new(StaticInner::new(asset, id));

        #[cfg(feature = "hot-reloading")]
        let inner: Box<dyn Any + Send + Sync> = if _reloadable {
            Box::new(DynamicInner::new(asset, id))
        } else {
            Box::new(StaticInner::new(asset, id))
        };

        CacheEntry(inner)
    }

    /// Returns a reference on the inner storage of the entry.
    #[inline]
    pub fn inner(&self) -> CacheEntryInner {
        CacheEntryInner(self.0.as_ref())
    }

    /// Consumes the `CacheEntry` and returns its inner value.
    #[inline]
    pub fn into_inner<T: 'static>(self) -> T {
        let _this = match self.0.downcast::<StaticInner<T>>() {
            Ok(inner) => return inner.value,
            Err(this) => this,
        };

        #[cfg(feature = "hot-reloading")]
        if let Ok(inner) = _this.downcast::<DynamicInner<T>>() {
            return inner.value.into_inner();
        }

        panic!("wrong handle type");
    }
}

impl fmt::Debug for CacheEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheEntry").finish()
    }
}

enum HandleInner<'a, T> {
    Static(&'a StaticInner<T>),
    #[cfg(feature = "hot-reloading")]
    Dynamic(&'a DynamicInner<T>),
}

impl<T> Clone for HandleInner<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for HandleInner<'_, T> {}

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
pub struct Handle<'a, T> {
    inner: HandleInner<'a, T>,
    last_reload: usize,
}

impl<'a, T> Handle<'a, T>
where
    T: 'static,
{
    /// Creates a new handle.
    ///
    /// `inner` must contain a `StaticInner<T>` or a `DynamicInner<T>`.
    fn new(inner: CacheEntryInner<'a>) -> Self {
        let inner = loop {
            if let Some(inner) = inner.0.downcast_ref::<StaticInner<T>>() {
                break HandleInner::Static(inner);
            }

            #[cfg(feature = "hot-reloading")]
            if let Some(inner) = inner.0.downcast_ref::<DynamicInner<T>>() {
                break HandleInner::Dynamic(inner);
            }

            panic!("wrong handle type");
        };

        let mut this = Self {
            inner,
            last_reload: 0,
        };
        this.reloaded();
        this
    }


    #[inline]
    pub(crate) fn either<U>(&self,
        on_static: impl FnOnce(&'a StaticInner<T>) -> U,
        _on_dynamic: impl FnOnce(&'a DynamicInner<T>) -> U,
    ) -> U {
        match &self.inner {
            HandleInner::Static(s) => on_static(s),
            #[cfg(feature = "hot-reloading")]
            HandleInner::Dynamic(s) => _on_dynamic(s),
        }
    }

    /// Locks the pointed asset for reading.
    ///
    /// If `T` implements `NotHotReloaded` or if hot-reloading is disabled, no
    /// reloading can occur so there is no actual lock. In these cases, calling
    /// this function is cheap and do not involve synchronisation.
    ///
    /// Returns a RAII guard which will release the lock once dropped.
    #[inline]
    pub fn read(&self) -> AssetGuard<'a, T> {
        let inner = self.either(
            |this| GuardInner::Ref(&this.value),
            |this| GuardInner::Guard(this.value.read()),
        );
        AssetGuard { inner }
    }

    /// Returns the id of the asset.
    ///
    /// Note that the lifetime of the returned `&str` is tied to that of the
    /// `AssetCache`, so it can outlive the handle.
    #[inline]
    pub fn id(&self) -> &'a str {
        self.either(|s| &s.id, |d| &d.id)
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
        let reloaded = self.either(
            |_| None,
            |this| Some(this.reload.load(Ordering::Acquire)),
        );

        match reloaded {
            None => false,
            Some(last_reload) => {
                let reloaded = last_reload > self.last_reload;
                self.last_reload = last_reload;
                reloaded
            },
        }
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
        self.either(
            |_| false,
            |this| this.reload_global.swap(false, Ordering::Acquire),
        )
    }

    /// Checks if the two handles refer to the same asset.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.either(
            |s1| other.either(|s2| std::ptr::eq(s1, s2), |_| false),
            |d1| other.either(|_| false, |d2| std::ptr::eq(d1, d2)),
        )
    }
}

impl<'a, A> Handle<'a, A>
where
    A: NotHotReloaded,
{
    /// Returns a reference to the underlying asset.
    ///
    /// This method only works if hot-reloading is disabled for the given type.
    #[inline]
    pub fn get(&self) -> &'a A {
        let _ = A::_CHECK_NOT_HOT_RELOADED;

        self.either(
            |this| &this.value,
            |_| panic!("`{}` implements `NotHotReloaded` but do not disable hot-reloading", type_name::<A>()),
        )
    }
}

impl<A> Handle<'_, A>
where
    A: Copy + 'static,
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
    A: Clone + 'static,
{
    /// Returns a clone of the inner asset.
    #[inline]
    pub fn cloned(self) -> A {
        self.read().clone()
    }
}

impl<A> Clone for Handle<'_, A> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for Handle<'_, A> {}

impl<T, U> PartialEq<Handle<'_, U>> for Handle<'_, T>
where
    T: PartialEq<U> + 'static,
    U: 'static,
{
    #[inline]
    fn eq(&self, other: &Handle<U>) -> bool {
        self.read().eq(&other.read())
    }
}

impl<A> Eq for Handle<'_, A> where A: Eq + 'static {}

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl<A> serde::Serialize for Handle<'_, A>
where
    A: serde::Serialize + 'static,
{
    #[inline]
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.read().serialize(s)
    }
}

impl<A> fmt::Debug for Handle<'_, A>
where
    A: fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle").field("value", &*self.read()).finish()
    }
}

pub enum GuardInner<'a, T> {
    Ref(&'a T),
    Guard(RwLockReadGuard<'a, T>),
}

/// RAII guard used to keep a read lock on an asset and release it when dropped.
///
/// This type is a smart pointer to type `A`.
///
/// It can be obtained by calling [`Handle::read`].
pub struct AssetGuard<'a, A> {
    inner: GuardInner<'a, A>,
}

impl<A> Deref for AssetGuard<'_, A> {
    type Target = A;

    #[inline]
    fn deref(&self) -> &A {
        match &self.inner {
            GuardInner::Ref(r) => r,
            GuardInner::Guard(g) => &g,
        }
    }
}

impl<A, U> AsRef<U> for AssetGuard<'_, A>
where
    A: AsRef<U>
{
    #[inline]
    fn as_ref(&self) -> &U {
        (&**self).as_ref()
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
