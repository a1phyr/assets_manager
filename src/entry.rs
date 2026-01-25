//! Definitions of cache entries

use crate::{Asset, SharedString, asset::Storable, key::Type, utils::RwLock};
use std::{
    any::{Any, TypeId},
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[cfg(feature = "hot-reloading")]
use crate::utils::RwLockReadGuard;

#[cfg(feature = "hot-reloading")]
unsafe fn swap_any(a: &mut dyn Any, b: &mut dyn Any) {
    debug_assert_eq!((a as &dyn Any).type_id(), (b as &dyn Any).type_id());
    debug_assert_eq!(
        std::alloc::Layout::for_value(a),
        std::alloc::Layout::for_value(b)
    );

    let len = std::mem::size_of_val(a);
    unsafe {
        std::ptr::swap_nonoverlapping(
            a as *mut dyn Any as *mut u8,
            b as *mut dyn Any as *mut u8,
            len,
        );
    }
}

#[allow(dead_code)]
pub(crate) struct Dynamic {
    typ: &'static Type,

    lock: RwLock<()>,
    reload_global: AtomicBool,
    reload: AtomicReloadId,
}

/// A handle on an asset.
///
/// It can be obtained through an [`AssetCache`].
///
/// If feature `hot-reloading` is used, this structure may wrap a `RwLock`, so
/// assets can be written to be reloaded. As such, any number of read guard can
/// exist at the same time, but none can exist while reloading an asset.
///
/// You can use this structure to reference an asset directly. However you can
/// only get *references* to this type, never own it. If you don't want to deal
/// with lifetimes, you can:
/// - Get a `&'static AssetCache` (eg with a `static` `LazyLock`), to get a
///   `'static` `Handle` refernce.
/// - Store the id of the asset and get it from the cache as needed.
/// - Get a [`ArcHandle`] through [`strong`] method.
///
/// [`AssetCache`]: `crate::AssetCache`
/// [`strong`]: `Self::strong`
pub struct Handle<T: ?Sized> {
    id: SharedString,
    type_id: TypeId,
    #[cfg(feature = "hot-reloading")]
    dynamic: Option<Dynamic>,
    value: UnsafeCell<T>,
}

unsafe impl<T: Sync + ?Sized> Sync for Handle<T> {}

impl<T: Storable> Handle<T> {
    fn new_static(id: SharedString, value: T) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
            #[cfg(feature = "hot-reloading")]
            dynamic: None,
            value: UnsafeCell::new(value),
        }
    }

    #[cfg(feature = "hot-reloading")]
    fn new_dynamic(id: SharedString, value: T) -> Self
    where
        T: Asset,
    {
        Self {
            id,
            type_id: TypeId::of::<T>(),
            dynamic: Some(Dynamic {
                typ: Type::of_asset::<T>(),
                lock: RwLock::new(()),
                reload_global: AtomicBool::new(false),
                reload: AtomicReloadId::new(),
            }),
            value: UnsafeCell::new(value),
        }
    }
}

impl UntypedHandle {
    #[cfg(feature = "hot-reloading")]
    pub(crate) fn write(&self, mut value: CacheEntry) {
        assert!(self.type_id == value.0.type_id);

        let Some(d) = &self.dynamic else {
            wrong_handle_type();
        };
        let storage = Arc::get_mut(&mut value.0).unwrap();

        unsafe {
            let _g = d.lock.write();
            swap_any(&mut *self.value.get(), storage.value.get_mut());
            d.reload.increment();
            d.reload_global.store(true, Ordering::Release);
        }
    }
}

/// An entry in the cache.
pub(crate) struct CacheEntry(Arc<UntypedHandle>);

impl CacheEntry {
    /// Creates a new `CacheEntry` containing an asset of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    #[inline]
    pub fn new<T: Asset>(asset: T, id: SharedString, _mutable: bool) -> Self {
        #[cfg(not(feature = "hot-reloading"))]
        let inner = Handle::new_static(id, asset);

        // Even if hot-reloading is enabled, we can avoid the lock in some cases.
        #[cfg(feature = "hot-reloading")]
        let inner = if T::HOT_RELOADED && _mutable {
            Handle::new_dynamic(id, asset)
        } else {
            Handle::new_static(id, asset)
        };

        CacheEntry(Arc::new(inner))
    }

    /// Creates a new `CacheEntry` containing a value of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    #[inline]
    pub fn new_any<T: Storable>(value: T, id: SharedString, _mutable: bool) -> Self {
        CacheEntry(Arc::new(Handle::new_static(id, value)))
    }

    #[inline]
    pub(crate) fn as_key(&self) -> (TypeId, &str) {
        (self.0.type_id, &self.0.id)
    }

    /// Returns a reference on the inner storage of the entry.
    #[inline]
    pub(crate) fn inner(&self) -> &UntypedHandle {
        &self.0
    }
}

impl fmt::Debug for CacheEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheEntry")
            .field("id", &self.0.id)
            .field("type_id", &self.0.type_id)
            .finish()
    }
}

/// A untyped handle on an asset.
///
/// This is an type-erased version of [`Handle`].
/// As with `dyn Any`, the underlying type can be queried at runtime.
pub type UntypedHandle = Handle<dyn Any + Send + Sync>;

impl UntypedHandle {
    #[inline]
    pub(crate) unsafe fn extend_lifetime<'a>(&self) -> &'a UntypedHandle {
        unsafe { &*(self as *const Self) }
    }

    /// Gets the `TypeId` of the underlying type.
    #[inline]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Returns `true` if the inner type is the same as T.
    #[inline]
    pub fn is<T: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<T>()
    }

    /// Returns a handle to the asset if it is of type `T`.
    #[inline]
    pub fn downcast_ref<T: Storable>(&self) -> Option<&Handle<T>> {
        if self.is::<T>() {
            unsafe { Some(&*(self as *const Self as *const Handle<T>)) }
        } else {
            None
        }
    }

    /// Like `downcast_ref`, but panics in the wrong type is given.
    #[inline]
    pub(crate) fn downcast_ref_ok<T: Storable>(&self) -> &Handle<T> {
        match self.downcast_ref() {
            Some(h) => h,
            None => wrong_handle_type(),
        }
    }
}

impl<T: ?Sized> Handle<T> {
    #[inline]
    fn either<'a, U>(
        &'a self,
        on_static: impl FnOnce() -> U,
        _on_dynamic: impl FnOnce(&'a Dynamic) -> U,
    ) -> U {
        #[cfg(feature = "hot-reloading")]
        if let Some(d) = &self.dynamic {
            return _on_dynamic(d);
        }

        on_static()
    }

    /// Locks the pointed asset for reading.
    ///
    /// If hot-reloading is disabled for `T` or globally, no reloading can occur
    /// so there is no actual lock. In these cases, calling this function does
    /// not involve synchronisation.
    ///
    /// Returns a RAII guard which will release the lock once dropped.
    #[inline]
    pub fn read(&self) -> AssetReadGuard<'_, T> {
        #[cfg(feature = "hot-reloading")]
        let guard = self.dynamic.as_ref().map(|d| d.lock.read());

        AssetReadGuard {
            value: unsafe { &*self.value.get() },
            #[cfg(feature = "hot-reloading")]
            guard,
        }
    }

    /// Returns the id of the asset.
    #[inline]
    pub fn id(&self) -> &SharedString {
        &self.id
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub(crate) fn typ(&self) -> Option<&'static Type> {
        self.either(|| None, |d| Some(d.typ))
    }

    /// Returns an untyped version of the handle.
    #[inline]
    pub fn as_untyped(&self) -> &UntypedHandle
    where
        T: Storable,
    {
        self
    }

    #[inline]
    fn as_arc(&self) -> ManuallyDrop<Arc<Handle<T>>> {
        // Safety: a `Handle<T>` is always in a `Arc`
        unsafe { ManuallyDrop::new(Arc::from_raw(self)) }
    }

    /// Make a `ArcHandle` that points to this handle.
    #[inline]
    pub fn strong(&self) -> ArcHandle<T> {
        ArcHandle(Arc::clone(&self.as_arc()))
    }

    /// Make a `WeakHandle` that points to this handle.
    #[inline]
    pub fn weak(&self) -> WeakHandle<T> {
        WeakHandle(Arc::downgrade(&self.as_arc()))
    }

    /// Gets the number of strong ([`ArcHandle`]) pointers to this allocation.
    #[inline]
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.as_arc())
    }

    /// Gets the number of [`WeakHandle`] pointers to this allocation.
    #[inline]
    pub fn weak_count(&self) -> usize {
        Arc::weak_count(&self.as_arc())
    }

    /// Returns a `ReloadWatcher` that can be used to check whether this asset
    /// was reloaded.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # cfg_if::cfg_if! { if #[cfg(feature = "hot-reloading")] {
    /// use assets_manager::{AssetCache, ReloadWatcher};
    ///
    /// let cache = AssetCache::new("assets")?;
    /// let asset = cache.load::<String>("common.some_text")?;
    /// let mut watcher = asset.reload_watcher();
    ///
    /// // The handle has just been created, so `reloaded` returns false
    /// assert!(!watcher.reloaded());
    ///
    /// loop {
    ///     if watcher.reloaded() {
    ///         println!("The asset was reloaded !")
    ///     }
    /// }
    ///
    /// # }}
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    #[inline]
    pub fn reload_watcher(&self) -> ReloadWatcher<'_> {
        ReloadWatcher::new(self.either(|| None, |d| Some(&d.reload)))
    }

    /// Returns the last `ReloadId` associated with this asset.
    ///
    /// It is only meaningful when compared to other `ReloadId`s returned by the
    /// same handle or to [`ReloadId::NEVER`].
    #[inline]
    pub fn last_reload_id(&self) -> ReloadId {
        self.either(|| ReloadId::NEVER, |this| this.reload.load())
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
            || false,
            |this| this.reload_global.swap(false, Ordering::Acquire),
        )
    }
}

impl<T> Handle<T>
where
    T: Copy,
{
    /// Returns a copy of the inner asset.
    ///
    /// This is functionnally equivalent to `cloned`, but it ensures that no
    /// expensive operation is used (eg if a type is refactored).
    #[inline]
    pub fn copied(&self) -> T {
        *self.read()
    }
}

impl<T> Handle<T>
where
    T: Clone,
{
    /// Returns a clone of the inner asset.
    #[inline]
    pub fn cloned(&self) -> T {
        self.read().clone()
    }
}

impl<T> fmt::Debug for Handle<T>
where
    T: fmt::Debug + ?Sized,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("id", self.id())
            .field("value", &&*self.read())
            .finish()
    }
}

/// A strong pointer to a handle.
///
/// Like a [`Arc`]`<`[`Handle`]`<T>>`, it deref to [`Handle`]`<T>`, can be
/// cloned and can be downgraded to [`WeakHandle`]`<T>`.
pub struct ArcHandle<T: ?Sized>(Arc<Handle<T>>);

impl ArcUntypedHandle {
    /// Attempt to downcast the handle to a concrete type.
    #[inline]
    pub fn downcast<T: 'static>(self) -> Result<ArcHandle<T>, Self> {
        if self.is::<T>() {
            unsafe {
                Ok(ArcHandle(Arc::from_raw(
                    Arc::into_raw(self.0) as *mut Handle<T>
                )))
            }
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized> Clone for ArcHandle<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: ?Sized> Deref for ArcHandle<T> {
    type Target = Handle<T>;

    #[inline]
    fn deref(&self) -> &Handle<T> {
        &self.0
    }
}

impl<T> fmt::Debug for ArcHandle<T>
where
    T: fmt::Debug + ?Sized,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// A weak pointer to a handle.
///
/// Like a [`Weak`]`<`[`Handle`]`<T>>`, it can be upgraded to an
/// [`ArcHandle`]`<T>`.
pub struct WeakHandle<T: ?Sized>(Weak<Handle<T>>);

impl<T> WeakHandle<T> {
    /// Constructs a new `WeakHandle<T>`, without allocating any memory.
    /// Calling [`upgrade`] on the return value always gives `None`.
    ///
    /// [`upgrade`]: WeakHandle::upgrade
    #[inline]
    pub const fn new() -> Self {
        Self(Weak::new())
    }
}

impl<T: ?Sized> WeakHandle<T> {
    /// Attempts to upgrade the `WeakHandle` to an `ArcHandle`.
    ///
    /// Returns `None` if the inner value has since been dropped.
    ///
    /// This is similar to [`Weak::upgrade`].
    #[inline]
    pub fn upgrade(&self) -> Option<ArcHandle<T>> {
        let arc = self.0.upgrade()?;
        Some(ArcHandle(arc))
    }

    /// Gets the number of strong (`Arc`) pointers pointing to this allocation.
    ///
    /// If `self` was created using [`WeakHandle::new`], this will return 0.
    #[inline]
    pub fn strong_count(&self) -> usize {
        Weak::strong_count(&self.0)
    }

    /// Gets an approximation of the number of `Weak` pointers pointing to this
    /// allocation.
    ///
    /// If `self` was created using [`WeakHandle::new`], or if there are no remaining
    /// strong pointers, this will return 0.
    #[inline]
    pub fn weak_count(&self) -> usize {
        Weak::weak_count(&self.0)
    }
}

impl<T> Default for WeakHandle<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> Clone for WeakHandle<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: ?Sized> fmt::Debug for WeakHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(WeakHandle)")
    }
}

/// An untyped version of [`ArcHandle`].
pub type ArcUntypedHandle = ArcHandle<dyn Any + Send + Sync>;
/// An untyped version of [`WeakHandle`].
pub type WeakUntypedHandle = WeakHandle<dyn Any + Send + Sync>;

/// RAII guard used to keep a read lock on an asset and release it when dropped.
///
/// This type is a smart pointer to type `T`.
///
/// It can be obtained by calling [`Handle::read`].
pub struct AssetReadGuard<'a, T: ?Sized> {
    value: &'a T,

    #[cfg(feature = "hot-reloading")]
    guard: Option<RwLockReadGuard<'a, ()>>,
}

impl<'a, T: ?Sized> AssetReadGuard<'a, T> {
    /// Make a new `AssetReadGuard` for a component of the locked data.
    pub fn map<U: ?Sized, F>(this: Self, f: F) -> AssetReadGuard<'a, U>
    where
        F: FnOnce(&T) -> &U,
    {
        AssetReadGuard {
            value: f(this.value),
            #[cfg(feature = "hot-reloading")]
            guard: this.guard,
        }
    }

    /// Attempts to make a new `AssetReadGuard` for a component of the locked data.
    ///
    /// Returns the original guard if the closure returns None.
    pub fn try_map<U: ?Sized, F>(this: Self, f: F) -> Result<AssetReadGuard<'a, U>, Self>
    where
        F: FnOnce(&T) -> Option<&U>,
    {
        match f(this.value) {
            Some(value) => Ok(AssetReadGuard {
                value,
                #[cfg(feature = "hot-reloading")]
                guard: this.guard,
            }),
            None => Err(this),
        }
    }
}

impl<'a> AssetReadGuard<'a, dyn Any> {
    /// Attempt to downcast the guard to a concrete type.
    pub fn downcast<T: Any>(self) -> Result<AssetReadGuard<'a, T>, Self> {
        Self::try_map(self, |x| x.downcast_ref())
    }
}

impl<'a> AssetReadGuard<'a, dyn Any + Send> {
    /// Attempt to downcast the guard to a concrete type.
    pub fn downcast<T: Any>(self) -> Result<AssetReadGuard<'a, T>, Self> {
        Self::try_map(self, |x| x.downcast_ref())
    }
}

impl<'a> AssetReadGuard<'a, dyn Any + Send + Sync> {
    /// Attempt to downcast the guard to a concrete type.
    pub fn downcast<T: Any>(self) -> Result<AssetReadGuard<'a, T>, Self> {
        Self::try_map(self, |x| x.downcast_ref())
    }
}

impl<T: ?Sized> Deref for AssetReadGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T, U> AsRef<U> for AssetReadGuard<'_, T>
where
    T: AsRef<U> + ?Sized,
{
    #[inline]
    fn as_ref(&self) -> &U {
        (**self).as_ref()
    }
}

impl<T> fmt::Display for AssetReadGuard<'_, T>
where
    T: fmt::Display + ?Sized,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<T> fmt::Debug for AssetReadGuard<'_, T>
where
    T: fmt::Debug + ?Sized,
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

        ReloadId::NEVER
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
/// by the same handle or to [`ReloadId::NEVER`].
///
/// They are useful when you cannot afford the associated lifetime of a
/// [`ReloadWatcher`]. In this case, you may be interested in using an
/// [`AtomicReloadId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReloadId(usize);

impl ReloadId {
    /// A `ReloadId` for values that were never updated.
    pub const NEVER: Self = Self(0);

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

impl Default for ReloadId {
    #[inline]
    fn default() -> Self {
        Self::NEVER
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
        Self::with_value(ReloadId::NEVER)
    }

    /// Creates a new atomic `ReloadId`, initialized with the given value.
    #[inline]
    pub const fn with_value(value: ReloadId) -> Self {
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

impl Default for AtomicReloadId {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cold]
#[track_caller]
fn wrong_handle_type() -> ! {
    panic!("wrong handle type");
}
