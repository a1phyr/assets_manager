use std::{
    fmt,
    hash,
    mem,
    ops::Deref,
    ptr,
};


#[cfg(feature = "parking_lot")]
pub use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
#[cfg(not(feature = "parking_lot"))]
pub use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};


/// `RwLock` from `parking_lot` and `std` have different APIs, so we use this
/// simple wrapper to easily permit both.
pub(crate) mod rwlock {
    use super::{RwLock, RwLockReadGuard, RwLockWriteGuard};

    /// Simple wrapper around RwLock reading method.
    #[inline]
    pub fn read<T: ?Sized>(this: &RwLock<T>) -> RwLockReadGuard<T> {
        #[cfg(feature = "parking_lot")]
        let guard = this.read();

        #[cfg(not(feature = "parking_lot"))]
        let guard = this.read().unwrap();

        guard
    }

    /// Simple wrapper around RwLock writing method.
    #[inline]
    pub fn write<T: ?Sized>(this: &RwLock<T>) -> RwLockWriteGuard<T> {
        #[cfg(feature = "parking_lot")]
        let guard = this.write();

        #[cfg(not(feature = "parking_lot"))]
        let guard = this.write().unwrap();

        guard
    }

    #[inline]
    pub fn get_mut<T: ?Sized>(this: &mut RwLock<T>) -> &mut T {
        #[cfg(feature = "parking_lot")]
        let guard = this.get_mut();

        #[cfg(not(feature = "parking_lot"))]
        let guard = this.get_mut().unwrap();

        guard
    }
}

/// This struct is used to store [`ContreteCacheEntry`] of different types in
/// the same container.
///
/// A [`ContreteCacheEntry`] can be transmuted in this struct without generic parameters.
///
/// The `repr(C)` ettribute ensures that the compiler doesn't change the layout
/// of the struct, so the data transmutation is legal. It is thus important to
/// keep the definitions of these structs in sync.
///
/// [`ContreteCacheEntry`]: struct.ContreteCacheEntry.html
#[repr(C)]
pub(crate) struct CacheEntry {
    /// A pointeur representing the `Box` contained by the underlying `ContreteCacheEntry`.
    data: *const RwLock<()>,

    /// A little hack to safely drop the underlying data without knowning its concrete type.
    drop_concrete: fn(&mut CacheEntry),
}

impl<'a, 'b> CacheEntry {
    #[inline]
    /// Create a new `CacheEntry` containing an asset of type `T`.
    ///
    /// The returned structure can safely use its methods with type parameter `T`.
    pub fn new<T: Send + Sync>(asset: T) -> Self {
        let concrete = ContreteCacheEntry {
            data: Box::new(RwLock::new(asset)),
            drop: ContreteCacheEntry::<T>::drop_data,
        };

        unsafe { mem::transmute(concrete) }
    }

    /// Get a reference to the underlying lock
    ///
    /// # Safety
    ///
    /// This function is unsafe in two ways:
    ///
    /// - The type parameter `T` has to be the same type as the actual type of the
    /// underlying data (ie this `CacheEntry` was created using `CacheEntry::new::<T>(...)`.
    /// - The lifetime of the return `AssetRefLock` is unbound, so you have to
    /// ensure that it won't outlive the given `CacheEntry`.
    #[inline]
    pub unsafe fn get_ref<T: Send + Sync>(&'a self) -> AssetRefLock<'b, T> {
        let concrete = {
            let ptr = self as *const CacheEntry as *const ContreteCacheEntry<T>;
            &*ptr
        };
        concrete.get_ref()
    }

    /// Write a value and a get reference to the underlying lock
    ///
    /// # Safety
    ///
    /// This function is unsafe in two ways:
    ///
    /// - The type parameter `T` has to be the same type as the actual type of the
    /// underlying data (ie this `CacheEntry` was created using `CacheEntry::new::<T>(...)`.
    /// - The lifetime of the return `AssetRefLock` is unbound, so you have to
    /// ensure that it won't outlive the given `CacheEntry`.
    #[inline]
    pub unsafe fn write<T: Send + Sync>(&'a self, asset: T) -> AssetRefLock<'b, T> {
        let lock = self.get_ref();
        let mut cached_guard = rwlock::write(&lock.data);
        *cached_guard = asset;
        drop(cached_guard);
        lock
    }
}

unsafe impl Send for CacheEntry {}
unsafe impl Sync for CacheEntry {}

impl fmt::Debug for CacheEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("CacheEntry")
    }
}

impl Drop for CacheEntry {
    fn drop(&mut self) {
        (self.drop_concrete)(self);
    }
}


#[repr(C)]
struct ContreteCacheEntry<T> {
    data: Box<RwLock<T>>,
    drop: fn(&mut CacheEntry),
}

impl<T: Send + Sync> ContreteCacheEntry<T> {
    fn drop_data(raw: &mut CacheEntry) {
        unsafe {
            let my_box = &mut raw.data as *mut *const RwLock<()> as *mut Box<RwLock<T>>;
            ptr::drop_in_place(my_box);
        }
    }

    #[inline]
    fn get_ref(&self) -> AssetRefLock<T> {
        AssetRefLock { data: &*self.data }
    }
}

impl<T> fmt::Debug for ContreteCacheEntry<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.data.read().fmt(f)
    }
}

/// A lock on an asset.
///
/// This structure wraps a RwLock, so assets can be written to be reloaded.
/// As such, any number of read guard can exist at the same time, but none
/// can exist while reloading an asset.
///
/// The type parameter `A` represents type of the locked asset.
#[derive(Clone)]
pub struct AssetRefLock<'a, A> {
    data: &'a RwLock<A>,
}

impl<A> AssetRefLock<'_, A> {
    /// Get a read lock on the pointed asset.
    ///
    /// Returns a RAII guard which will release the lock once dropped.
    #[inline]
    pub fn read(&self) -> AssetRef<'_, A> {
        AssetRef {
            guard: rwlock::read(self.data),
        }
    }

    /// Check if to assets a are the same
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.data, other.data)
    }
}

impl<A> hash::Hash for AssetRefLock<'_, A>
where
    A: hash::Hash,
{
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        rwlock::read(self.data).hash(state);
    }
}

impl<A> fmt::Debug for AssetRefLock<'_, A>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetRefLock").field("data", &*rwlock::read(&self.data)).finish()
    }
}

/// RAII guard used to keep a read lock on an asset and release it when dropped.
///
/// It can be obtained by calling [`AssetRefLock::read`].
///
/// [`AssetRefLock::read`]: struct.AssetRefLock.html#method.read
pub struct AssetRef<'a, A> {
    guard: RwLockReadGuard<'a, A>,
}

impl<A> Deref for AssetRef<'_, A> {
    type Target = A;

    #[inline]
    fn deref(&self) -> &A {
        &self.guard
    }
}

impl<A> fmt::Display for AssetRef<'_, A>
where
    A: fmt::Display,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<A> fmt::Debug for AssetRef<'_, A>
where
    A: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
