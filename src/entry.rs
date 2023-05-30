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
    reload: AtomicReloadId,
}

#[cfg(feature = "hot-reloading")]
impl<T> DynamicInner<T> {
    #[inline]
    fn new(value: T, id: SharedString) -> Self {
        Self {
            id,
            value: RwLock::new(value),
            reload: AtomicReloadId::new(),
            reload_global: AtomicBool::new(false),
        }
    }

    pub fn write(&self, value: T) {
        let mut data = self.value.write();
        *data = value;
        self.reload.increment();
        self.reload_global.store(true, Ordering::Release);
    }
}

/// An entry in the cache.
pub struct CacheEntry(Box<dyn Any + Send + Sync>);

impl CacheEntry {
    /// Creates a new `CacheEntry` containing an asset of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    #[inline]
    pub fn new<T: Storable>(asset: T, id: SharedString, _mutable: impl FnOnce() -> bool) -> Self {
        #[cfg(not(feature = "hot-reloading"))]
        let inner = Box::new(StaticInner::new(asset, id));

        // Even if hot-reloading is enabled, we can avoid the lock in some cases.
        #[cfg(feature = "hot-reloading")]
        let inner: Box<dyn Any + Send + Sync> = if T::HOT_RELOADED && _mutable() {
            Box::new(DynamicInner::new(asset, id))
        } else {
            Box::new(StaticInner::new(asset, id))
        };

        CacheEntry(inner)
    }

    /// Returns a reference on the inner storage of the entry.
    #[inline]
    pub(crate) fn inner(&self) -> UntypedHandle {
        UntypedHandle(self.0.as_ref())
    }

    /// Consumes the `CacheEntry` and returns its inner value.
    #[inline]
    pub fn into_inner<T: Storable>(self) -> (T, SharedString) {
        #[allow(unused_mut)]
        let mut this = self.0;

        #[cfg(feature = "hot-reloading")]
        if T::HOT_RELOADED {
            match this.downcast::<DynamicInner<T>>() {
                Ok(inner) => return (inner.value.into_inner(), inner.id),
                Err(t) => this = t,
            }
        }

        if let Ok(inner) = this.downcast::<StaticInner<T>>() {
            return (inner.value, inner.id);
        }

        wrong_handle_type()
    }
}

impl fmt::Debug for CacheEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheEntry").finish()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct UntypedHandle<'a>(&'a (dyn Any + Send + Sync));

impl<'a> UntypedHandle<'a> {
    #[inline]
    pub(crate) unsafe fn extend_lifetime<'b>(self) -> UntypedHandle<'b> {
        let inner = &*(self.0 as *const (dyn Any + Send + Sync));
        UntypedHandle(inner)
    }

    #[inline]
    pub fn try_downcast<T: Storable>(self) -> Option<Handle<'a, T>> {
        Handle::new(self)
    }

    #[inline]
    pub fn downcast<T: Storable>(self) -> Handle<'a, T> {
        match self.try_downcast() {
            Some(h) => h,
            None => wrong_handle_type(),
        }
    }
}

impl fmt::Debug for UntypedHandle<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UntypedHandle").finish()
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
    fn new(inner: UntypedHandle<'a>) -> Option<Self>
    where
        T: Storable,
    {
        #[allow(clippy::never_loop)]
        let inner = loop {
            #[cfg(feature = "hot-reloading")]
            if T::HOT_RELOADED {
                if let Some(inner) = inner.0.downcast_ref::<DynamicInner<T>>() {
                    break HandleInner::Dynamic(inner);
                }
            }

            if let Some(inner) = inner.0.downcast_ref::<StaticInner<T>>() {
                break HandleInner::Static(inner);
            }

            return None;
        };

        Some(Handle { inner })
    }

    #[inline]
    fn either<U>(
        &self,
        on_static: impl FnOnce(&'a StaticInner<T>) -> U,
        _on_dynamic: impl FnOnce(&'a DynamicInner<T>) -> U,
    ) -> U {
        match self.inner {
            HandleInner::Static(s) => on_static(s),
            #[cfg(feature = "hot-reloading")]
            HandleInner::Dynamic(s) => _on_dynamic(s),
        }
    }

    #[inline]
    #[cfg(feature = "hot-reloading")]
    pub(crate) fn write(&self, value: T) {
        self.either(|_| wrong_handle_type(), |this| this.write(value))
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
        let inner = match self.inner {
            HandleInner::Static(this) => GuardInner::Ref(&this.value),
            #[cfg(feature = "hot-reloading")]
            HandleInner::Dynamic(this) => GuardInner::Guard(this.value.read()),
        };
        AssetGuard { inner }
    }

    /// Returns the id of the asset.
    ///
    /// Note that the lifetime of the returned `&str` is tied to that of the
    /// `AssetCache`, so it can outlive the handle.
    #[inline]
    pub fn id(&self) -> &'a SharedString {
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
        self.either(|_| ReloadId(0), |this| this.reload.load())
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
    #[allow(clippy::let_unit_value)]
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
        (**self).as_ref()
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
    reload_id: &'a AtomicReloadId,
    last_reload_id: ReloadId,
}

#[cfg(feature = "hot-reloading")]
impl<'a> ReloadWatcherInner<'a> {
    #[inline]
    fn new(reload_id: &'a AtomicReloadId) -> Self {
        Self {
            reload_id,
            last_reload_id: reload_id.load(),
        }
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
    fn new(_reload_id: Option<&'a AtomicReloadId>) -> Self {
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
            let new_id = inner.reload_id.load();
            return inner.last_reload_id.update(new_id);
        }

        false
    }

    /// Returns the last `ReloadId` associated with this asset.
    #[inline]
    pub fn last_reload_id(&self) -> ReloadId {
        #[cfg(feature = "hot-reloading")]
        if let Some(inner) = &self.inner {
            return inner.reload_id.load();
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
/// [`ReloadWatcher`]. In this case, you may be interested in using an
/// [`AtomicReloadId`].
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

/// A [`ReloadId`] that can be shared between threads.
///
/// This type is useful when one cannot afford the associated lifetime of
/// [`ReloadWatcher`] and is cheaper than a `Mutex<ReloadId>`.
///
/// `update` method is enough to satisfy most needs, but this type exposes more
/// primitive operations too.
#[derive(Debug)]
pub struct AtomicReloadId(AtomicUsize);

impl AtomicReloadId {
    /// Creates a new atomic `ReloadId`.
    #[inline]
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    /// Creates a new atomic `ReloadId`, initialized with the given value.
    #[inline]
    pub fn with_value(value: ReloadId) -> Self {
        Self(AtomicUsize::new(value.0))
    }

    /// Updates `self` if the argument if the argument is newer. Returns `true`
    /// if `self` was updated.
    #[inline]
    pub fn update(&self, new: ReloadId) -> bool {
        new > self.fetch_max(new)
    }

    /// Loads the inner `ReloadId`.
    #[inline]
    pub fn load(&self) -> ReloadId {
        ReloadId(self.0.load(Ordering::Acquire))
    }

    /// Stores a `ReloadId`.
    #[inline]
    pub fn store(&self, new: ReloadId) {
        self.0.store(new.0, Ordering::Release)
    }

    #[inline]
    #[cfg(feature = "hot-reloading")]
    fn increment(&self) {
        self.0.fetch_add(1, Ordering::Release);
    }

    /// Stores a `ReloadId`, returning the previous one.
    #[inline]
    pub fn swap(&self, new: ReloadId) -> ReloadId {
        ReloadId(self.0.swap(new.0, Ordering::AcqRel))
    }

    /// Stores the maximum of the two `ReloadId`, returning the previous one.
    #[inline]
    pub fn fetch_max(&self, new: ReloadId) -> ReloadId {
        ReloadId(self.0.fetch_max(new.0, Ordering::AcqRel))
    }
}

#[cold]
#[track_caller]
fn wrong_handle_type() -> ! {
    panic!("wrong handle type");
}
