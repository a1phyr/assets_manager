//! Definitions of cache entries

use std::{
    any::{type_name, Any, TypeId},
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

/// The representation of an asset whose value can be updated (eg through
/// hot-reloading).
#[allow(dead_code)]
pub(crate) struct DynamicStorage<T> {
    value: RwLock<T>,
    reload_global: AtomicBool,
    reload: AtomicReloadId,
}

#[cfg(feature = "hot-reloading")]
impl<T> DynamicStorage<T> {
    #[inline]
    fn new(value: T) -> Self {
        Self {
            value: RwLock::new(value),
            reload_global: AtomicBool::new(false),
            reload: AtomicReloadId::new(),
        }
    }

    #[inline]
    pub fn write(&self, value: T) {
        let mut data = self.value.write();
        *data = value;
        self.reload.increment();
        self.reload_global.store(true, Ordering::Release);
    }
}

enum EntryKind<T> {
    Static(T),
    #[cfg(feature = "hot-reloading")]
    Dynamic(DynamicStorage<T>),
}

impl<T> EntryKind<T> {
    #[inline]
    fn into_inner(self) -> T {
        match self {
            EntryKind::Static(value) => value,
            #[cfg(feature = "hot-reloading")]
            EntryKind::Dynamic(inner) => inner.value.into_inner(),
        }
    }
}

trait Storage: Send + Sync {
    fn read(&self) -> AssetReadGuard<'_, dyn Any + Send + Sync>;

    #[cfg(feature = "hot-reloading")]
    fn write(&self, asset: CacheEntry) -> SharedString;
}

impl<T: Storable> Storage for EntryKind<T> {
    fn read(&self) -> AssetReadGuard<'_, dyn Any + Send + Sync> {
        let inner = match self {
            EntryKind::Static(value) => GuardInner::Ref(value as &(dyn Any + Send + Sync)),
            #[cfg(feature = "hot-reloading")]
            EntryKind::Dynamic(inner) => {
                let rw: &RwLock<dyn Any + Send + Sync> = &inner.value;
                GuardInner::Guard(rw.read())
            }
        };
        AssetReadGuard { inner }
    }

    #[cfg(feature = "hot-reloading")]
    fn write(&self, asset: CacheEntry) -> SharedString {
        let (asset, id) = asset.into_inner();
        match self {
            EntryKind::Static(_) => wrong_handle_type(),
            EntryKind::Dynamic(d) => d.write(asset),
        }
        id
    }
}

struct RawEntry<T: ?Sized> {
    id: SharedString,
    type_id: TypeId,
    kind: T,
}

type Entry<T> = RawEntry<EntryKind<T>>;
type UntypedEntry = RawEntry<dyn Storage>;

impl<T: Storable> Entry<T> {
    fn handle(&self) -> &Handle<T> {
        unsafe { &*(self as *const Self as *const Handle<T>) }
    }

    fn untyped_handle(&self) -> &UntypedHandle {
        unsafe { &*(self as *const Self as *const UntypedEntry as *const UntypedHandle) }
    }
}

impl UntypedEntry {
    fn is<T: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<T>()
    }

    fn downcast_ref<T: 'static>(&self) -> Option<&Entry<T>> {
        if self.is::<T>() {
            unsafe { Some(&*(self as *const Self as *const Entry<T>)) }
        } else {
            None
        }
    }

    fn downcast<T: 'static>(self: Box<Self>) -> Result<Box<Entry<T>>, Box<Self>> {
        if self.is::<T>() {
            unsafe { Ok(Box::from_raw(Box::into_raw(self) as *mut Entry<T>)) }
        } else {
            Err(self)
        }
    }
}

/// An entry in the cache.
pub struct CacheEntry(Box<UntypedEntry>);

impl CacheEntry {
    /// Creates a new `CacheEntry` containing an asset of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    #[inline]
    pub fn new<T: Storable>(asset: T, id: SharedString, _mutable: impl FnOnce() -> bool) -> Self {
        #[cfg(not(feature = "hot-reloading"))]
        let kind = EntryKind::Static(asset);

        // Even if hot-reloading is enabled, we can avoid the lock in some cases.
        #[cfg(feature = "hot-reloading")]
        let kind = if T::HOT_RELOADED && _mutable() {
            EntryKind::Dynamic(DynamicStorage::new(asset))
        } else {
            EntryKind::Static(asset)
        };

        let type_id = TypeId::of::<T>();
        CacheEntry(Box::new(Entry { id, type_id, kind }))
    }

    /// Returns a reference on the inner storage of the entry.
    #[inline]
    pub(crate) fn inner(&self) -> &UntypedHandle {
        unsafe { &*(&*self.0 as *const _ as *const UntypedHandle) }
    }

    /// Consumes the `CacheEntry` and returns its inner value.
    #[inline]
    pub fn into_inner<T: Storable>(self) -> (T, SharedString) {
        if let Ok(storage) = self.0.downcast() {
            return (storage.kind.into_inner(), storage.id);
        }

        wrong_handle_type()
    }
}

impl fmt::Debug for CacheEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheEntry").finish()
    }
}

/// A untyped handle on an asset.
///
/// This is an type-erased version of [`Handle`].
/// As with `dyn Any`, the underlying type can be queried at runtime.
#[repr(transparent)]
pub struct UntypedHandle {
    inner: UntypedEntry,
}

impl UntypedHandle {
    /// Locks the pointed asset for reading.
    ///
    /// If `T` implements `NotHotReloaded` or if hot-reloading is disabled, no
    /// reloading can occur so there is no actual lock. In these cases, calling
    /// this function does not involve synchronisation.
    ///
    /// Returns a RAII guard which will release the lock once dropped.
    #[inline]
    pub fn read(&self) -> AssetReadGuard<'_, dyn Any + Send + Sync> {
        self.inner.kind.read()
    }

    /// Returns the id of the asset.
    #[inline]
    pub fn id(&self) -> &SharedString {
        &self.inner.id
    }

    #[inline]
    pub(crate) unsafe fn extend_lifetime<'a>(&self) -> &'a UntypedHandle {
        &*(self as *const Self)
    }

    /// Returns `true` if the inner type is the same as T.
    #[inline]
    pub fn is<T: 'static>(&self) -> bool {
        self.inner.is::<T>()
    }

    /// Returns a handle to the asset if it is of type `T`.
    #[inline]
    pub fn downcast_ref<T: Storable>(&self) -> Option<&Handle<T>> {
        let entry = self.inner.downcast_ref()?;
        Some(entry.handle())
    }

    /// Like `downcast_ref`, but panics in the wrong type is given.
    #[inline]
    pub(crate) fn downcast_ref_ok<T: Storable>(&self) -> &Handle<T> {
        match self.downcast_ref() {
            Some(h) => h,
            None => wrong_handle_type(),
        }
    }

    #[cfg(feature = "hot-reloading")]
    pub(crate) fn write(&self, asset: CacheEntry) -> SharedString {
        self.inner.kind.write(asset)
    }
}

impl fmt::Debug for UntypedHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UntypedHandle")
            .field("id", self.id())
            .finish_non_exhaustive()
    }
}

/// A handle on an asset.
///
/// Such a handle can be used to get access to an asset of type `T`. It is
/// generally obtained by call `AssetCache::load` and its variants.
///
/// If feature `hot-reloading` is used, this structure wraps a RwLock, so
/// assets can be written to be reloaded. As such, any number of read guard can
/// exist at the same time, but none can exist while reloading an asset (when
/// calling `AssetCache::hot_reload`).
///
/// You can use thus structure to store a reference to an asset.
/// However it is generally easier to work with `'static` data. For more
/// information, see [top-level documentation](crate#getting-owned-data).
#[repr(transparent)]
pub struct Handle<T> {
    inner: Entry<T>,
}

impl<T> Handle<T> {
    #[inline]
    fn either<'a, U>(
        &'a self,
        on_static: impl FnOnce(&'a T) -> U,
        _on_dynamic: impl FnOnce(&'a DynamicStorage<T>) -> U,
    ) -> U {
        match &self.inner.kind {
            EntryKind::Static(s) => on_static(s),
            #[cfg(feature = "hot-reloading")]
            EntryKind::Dynamic(s) => _on_dynamic(s),
        }
    }

    /// Locks the pointed asset for reading.
    ///
    /// If `T` implements `NotHotReloaded` or if hot-reloading is disabled, no
    /// reloading can occur so there is no actual lock. In these cases, calling
    /// this function is cheap and does not involve synchronisation.
    ///
    /// Returns a RAII guard which will release the lock once dropped.
    #[inline]
    pub fn read(&self) -> AssetReadGuard<'_, T> {
        let inner = match &self.inner.kind {
            EntryKind::Static(value) => GuardInner::Ref(value),
            #[cfg(feature = "hot-reloading")]
            EntryKind::Dynamic(inner) => GuardInner::Guard(inner.value.read()),
        };
        AssetReadGuard { inner }
    }

    /// Returns the id of the asset.
    #[inline]
    pub fn id(&self) -> &SharedString {
        &self.inner.id
    }

    /// Returns an untyped version of the handle.
    #[inline]
    pub fn as_untyped(&self) -> &UntypedHandle
    where
        T: Storable,
    {
        self.inner.untyped_handle()
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
    pub fn reload_watcher(&self) -> ReloadWatcher<'_> {
        ReloadWatcher::new(self.either(|_| None, |d| Some(&d.reload)))
    }

    /// Returns the last `ReloadId` associated with this asset.
    ///
    /// It is only meaningful when compared to other `ReloadId`s returned by the
    /// same handle or to [`ReloadId::NEVER`].
    #[inline]
    pub fn last_reload_id(&self) -> ReloadId {
        self.either(|_| ReloadId::NEVER, |this| this.reload.load())
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
}

impl<A> Handle<A>
where
    A: NotHotReloaded,
{
    /// Returns a reference to the underlying asset.
    ///
    /// This method only works if hot-reloading is disabled for the given type.
    #[inline]
    #[allow(clippy::let_unit_value)]
    pub fn get(&self) -> &A {
        let _ = A::_CHECK_NOT_HOT_RELOADED;

        self.either(
            |value| value,
            |_| {
                panic!(
                    "`{}` implements `NotHotReloaded` but do not disable hot-reloading",
                    type_name::<A>()
                )
            },
        )
    }
}

impl<A> Handle<A>
where
    A: Copy,
{
    /// Returns a copy of the inner asset.
    ///
    /// This is functionnally equivalent to `cloned`, but it ensures that no
    /// expensive operation is used (eg if a type is refactored).
    #[inline]
    pub fn copied(&self) -> A {
        *self.read()
    }
}

impl<A> Handle<A>
where
    A: Clone,
{
    /// Returns a clone of the inner asset.
    #[inline]
    pub fn cloned(&self) -> A {
        self.read().clone()
    }
}

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl<A> serde::Serialize for Handle<A>
where
    A: serde::Serialize,
{
    #[inline]
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.read().serialize(s)
    }
}

impl<A> fmt::Debug for Handle<A>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("id", self.id())
            .field("value", &*self.read())
            .finish()
    }
}

pub enum GuardInner<'a, T: ?Sized> {
    Ref(&'a T),
    #[cfg(feature = "hot-reloading")]
    Guard(RwLockReadGuard<'a, T>),
}

/// RAII guard used to keep a read lock on an asset and release it when dropped.
///
/// This type is a smart pointer to type `A`.
///
/// It can be obtained by calling [`Handle::read`].
pub struct AssetReadGuard<'a, A: ?Sized> {
    inner: GuardInner<'a, A>,
}

impl<A: ?Sized> Deref for AssetReadGuard<'_, A> {
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

impl<A, U> AsRef<U> for AssetReadGuard<'_, A>
where
    A: AsRef<U> + ?Sized,
{
    #[inline]
    fn as_ref(&self) -> &U {
        (**self).as_ref()
    }
}

impl<A> fmt::Display for AssetReadGuard<'_, A>
where
    A: fmt::Display + ?Sized,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<A> fmt::Debug for AssetReadGuard<'_, A>
where
    A: fmt::Debug + ?Sized,
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

#[cold]
#[track_caller]
fn wrong_handle_type() -> ! {
    panic!("wrong handle type");
}
