//! Definitions of cache entries

use std::{
    any::{type_name, Any},
    fmt,
    marker::PhantomData,
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{
    asset::{NotHotReloaded, Storable},
    utils::RwLock,
    SharedString,
};

#[cfg(feature = "hot-reloading")]
use crate::utils::RwLockReadGuard;

/// The representation of an asset whose value cannot change.
pub(crate) struct StaticInner<T> {
    id: SharedString,
    value: T,
}

impl<T> StaticInner<T> {
    #[inline]
    fn new(value: T, id: SharedString) -> Self {
        Self { id, value }
    }
}

/// The representation of an asset whose value can be updated (eg through
/// hot-reloading).
#[allow(dead_code)]
pub(crate) struct DynamicInner<T> {
    id: SharedString,
    value: RwLock<T>,
    reload_global: AtomicBool,
    reload: AtomicUsize,
}

impl<T> DynamicInner<T> {
    #[inline]
    #[cfg(feature = "hot-reloading")]
    fn new(value: T, id: SharedString) -> Self {
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
pub struct CacheEntry(pub Box<dyn Any + Send + Sync>);

impl CacheEntry {
    /// Creates a new `CacheEntry` containing an asset of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    #[inline]
    pub fn new<T: Storable>(asset: T, id: SharedString) -> Self {
        #[cfg(not(feature = "hot-reloading"))]
        let inner = Box::new(StaticInner::new(asset, id));

        #[cfg(feature = "hot-reloading")]
        let inner: Box<dyn Any + Send + Sync> = if T::HOT_RELOADED {
            Box::new(DynamicInner::new(asset, id))
        } else {
            Box::new(StaticInner::new(asset, id))
        };

        CacheEntry(inner)
    }

    /// Returns a reference on the inner storage of the entry.
    #[inline]
    pub(crate) fn inner(&self) -> CacheEntryInner {
        CacheEntryInner(self.0.as_ref())
    }

    /// Consumes the `CacheEntry` and returns its inner value.
    #[inline]
    pub fn into_inner<T: 'static>(self) -> (T, SharedString) {
        let _this = match self.0.downcast::<StaticInner<T>>() {
            Ok(inner) => return (inner.value, inner.id),
            Err(this) => this,
        };

        #[cfg(feature = "hot-reloading")]
        if let Ok(inner) = _this.downcast::<DynamicInner<T>>() {
            return (inner.value.into_inner(), inner.id);
        }

        wrong_handle_type()
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
/// information, see [top-level documentation](crate#getting-owned-data).
pub struct Handle<'a, T> {
    inner: HandleInner<'a, T>,
}

impl<'a, T> Handle<'a, T> {
    /// Creates a new handle.
    ///
    /// `inner` must contain a `StaticInner<T>` or a `DynamicInner<T>`.
    fn new(inner: CacheEntryInner<'a>) -> Self
    where
        T: 'static,
    {
        let inner = loop {
            if let Some(inner) = inner.0.downcast_ref::<StaticInner<T>>() {
                break HandleInner::Static(inner);
            }

            #[cfg(feature = "hot-reloading")]
            if let Some(inner) = inner.0.downcast_ref::<DynamicInner<T>>() {
                break HandleInner::Dynamic(inner);
            }

            wrong_handle_type()
        };

        Handle { inner }
    }

    #[inline]
    pub(crate) fn either<U>(
        &self,
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
            #[cfg(feature = "hot-reloading")]
            |this| GuardInner::Guard(this.value.read()),
            #[cfg(not(feature = "hot-reloading"))]
            |_| unimplemented!(),
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

    /// Returns a `ReloadWatcher` that can be used to check whether this asset
    /// was reloaded.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # cfg_if::cfg_if! { if #[cfg(feature = "hot-reloading")] {
    /// use assets_manager::{Asset, AssetCache, ReloadWatcher};
    ///
    /// let cache = AssetCache::new("assets")?;
    /// let asset = cache.load::<String>("common.some_text")?;
    /// let mut watcher = asset.reload_watcher();
    ///
    /// // The handle has just been created, so `reloaded` returns false
    /// assert!(!watcher.reloaded());
    ///
    /// loop {
    ///     cache.hot_reload();
    ///
    ///     if watcher.reloaded() {
    ///         println!("The asset was reloaded !")
    ///     }
    ///
    ///     // Calling `reloaded` once more returns false: the asset has not
    ///     // been reloaded since last call to `reloaded`
    ///     assert!(!watcher.reloaded());
    /// }
    ///
    /// # }}
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    #[inline]
    pub fn reload_watcher(&self) -> ReloadWatcher<'a> {
        ReloadWatcher::new(self.either(|_| None, |d| Some(&d.reload)))
    }

    /// Returns the last `ReloadId` associated with this asset.
    ///
    /// It is only meaningful when compared to other `ReloadId`s returned by the
    /// [same handle](`Self::same_handle`).
    #[inline]
    pub fn last_reload_id(&self) -> ReloadId {
        let id = self.either(|_| 0, |this| this.reload.load(Ordering::Acquire));
        ReloadId(id)
    }

    /// Returns `true` if the asset has been reloaded since last call to this
    /// method with **any** handle on this asset.
    ///
    /// Note that this method and [`reload_watcher`] are totally independant,
    /// and the result of the two functions do not depend on whether the other
    /// was called.
    ///
    /// [`reload_watcher`]: Self::reload_watcher
    #[inline]
    pub fn reloaded_global(&self) -> bool {
        self.either(
            |_| false,
            |this| this.reload_global.swap(false, Ordering::Acquire),
        )
    }

    /// Checks if the two handles refer to the same asset.
    #[inline]
    pub fn same_handle(&self, other: &Self) -> bool {
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
            |_| {
                panic!(
                    "`{}` implements `NotHotReloaded` but do not disable hot-reloading",
                    type_name::<A>()
                )
            },
        )
    }
}

impl<A> Handle<'_, A>
where
    A: Copy,
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
    A: Clone,
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
    T: PartialEq<U>,
{
    #[inline]
    fn eq(&self, other: &Handle<U>) -> bool {
        self.read().eq(&other.read())
    }
}

impl<A> Eq for Handle<'_, A> where A: Eq {}

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl<A> serde::Serialize for Handle<'_, A>
where
    A: serde::Serialize,
{
    #[inline]
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.read().serialize(s)
    }
}

impl<A> fmt::Debug for Handle<'_, A>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("value", &*self.read())
            .finish()
    }
}

pub enum GuardInner<'a, T> {
    Ref(&'a T),
    #[cfg(feature = "hot-reloading")]
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
            #[cfg(feature = "hot-reloading")]
            GuardInner::Guard(g) => g,
        }
    }
}

impl<A, U> AsRef<U> for AssetGuard<'_, A>
where
    A: AsRef<U>,
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

#[cfg(feature = "hot-reloading")]
#[derive(Debug, Clone, Copy)]
struct ReloadWatcherInner<'a> {
    reload_id: &'a AtomicUsize,
    last_reload_id: ReloadId,
}

#[cfg(feature = "hot-reloading")]
impl<'a> ReloadWatcherInner<'a> {
    #[inline]
    fn new(reload_id: &'a AtomicUsize) -> Self {
        Self {
            reload_id,
            last_reload_id: ReloadId(reload_id.load(Ordering::Relaxed)),
        }
    }

    #[inline]
    pub fn last_reload_id(&self) -> ReloadId {
        ReloadId(self.reload_id.load(Ordering::Acquire))
    }
}

/// A watcher that can tell when an asset is reloaded.
///
/// Each `ReloadWatcher` is associated to a single asset in a cache.
///
/// It can be obtained with [`Handle::reload_watcher`].
#[derive(Debug, Clone, Copy)]
pub struct ReloadWatcher<'a> {
    #[cfg(feature = "hot-reloading")]
    inner: Option<ReloadWatcherInner<'a>>,
    _private: PhantomData<&'a ()>,
}

impl<'a> ReloadWatcher<'a> {
    #[inline]
    fn new(_reload_id: Option<&'a AtomicUsize>) -> Self {
        #[cfg(feature = "hot-reloading")]
        let inner = _reload_id.map(ReloadWatcherInner::new);
        Self {
            #[cfg(feature = "hot-reloading")]
            inner,
            _private: PhantomData,
        }
    }

    /// Returns `true` if the watched asset was reloaded since the last call to
    /// this function.
    #[inline]
    pub fn reloaded(&mut self) -> bool {
        #[cfg(feature = "hot-reloading")]
        if let Some(inner) = &mut self.inner {
            let new_id = inner.last_reload_id();
            return inner.last_reload_id.update(new_id);
        }

        false
    }

    /// Returns the last `ReloadId` associated with this asset.
    #[inline]
    pub fn last_reload_id(&self) -> ReloadId {
        #[cfg(feature = "hot-reloading")]
        if let Some(inner) = &self.inner {
            return inner.last_reload_id();
        }

        ReloadId(0)
    }
}

impl Default for ReloadWatcher<'_> {
    /// Returns a `ReloadWatcher` that never gets updated.
    #[inline]
    fn default() -> Self {
        Self::new(None)
    }
}

/// An id to know when an asset is reloaded.
///
/// Each time an asset is reloaded, it gets a new `ReloadId` that compares
/// superior to the previous one.
///
/// `ReloadId`s are only meaningful when compared to other `ReloadId`s returned
/// by the [same handle](`Handle::same_handle`).
///
/// They are useful when you cannot afford the associated lifetime of a
/// `ReloadWatcher`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReloadId(usize);

impl ReloadId {
    /// Updates `self` if the argument if the argument is newer. Returns `true`
    /// if `self` was updated.
    #[inline]
    pub fn update(&mut self, new: ReloadId) -> bool {
        let newer = new > *self;
        if newer {
            *self = new;
        }
        newer
    }
}

#[cold]
fn wrong_handle_type() -> ! {
    panic!("wrong handle type");
}
