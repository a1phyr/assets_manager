#[cfg(feature = "parking_lot")]
use parking_lot as sync;
#[cfg(not(feature = "parking_lot"))]
use std::sync;

pub(crate) use sync::{RwLockReadGuard, RwLockWriteGuard};


#[cfg(feature = "parking_lot")]
#[inline]
fn wrap<T>(param: T) -> T {
    param
}

#[cfg(not(feature = "parking_lot"))]
#[inline]
fn wrap<T>(param: sync::LockResult<T>) -> T {
    // Just ignore poison errors
    param.unwrap_or_else(sync::PoisonError::into_inner)
}


/// `RwLock` from `parking_lot` and `std` have different APIs, so we use this
/// simple wrapper to easily permit both.
pub(crate) struct RwLock<T: ?Sized>(sync::RwLock<T>);

impl<T> RwLock<T> {
    #[inline]
    pub fn new(inner: T) -> Self {
        Self(sync::RwLock::new(inner))
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn into_inner(self) -> T {
        wrap(self.0.into_inner())
    }
}

impl<T: ?Sized> RwLock<T> {
    #[inline]
    pub fn read(&self) -> RwLockReadGuard<T> {
        wrap(self.0.read())
    }

    #[inline]
    pub fn write(&self) -> RwLockWriteGuard<T> {
        wrap(self.0.write())
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        wrap(self.0.get_mut())
    }
}


#[cfg(feature = "hot-reloading")]
pub(crate) struct Mutex<T: ?Sized>(sync::Mutex<T>);

#[cfg(feature = "hot-reloading")]
impl<T> Mutex<T> {
    #[inline]
    pub fn new(inner: T) -> Self {
        Self(sync::Mutex::new(inner))
    }
}

#[cfg(feature = "hot-reloading")]
impl<T: ?Sized> Mutex<T> {
    #[inline]
    pub fn lock(&self) -> sync::MutexGuard<T> {
        wrap(self.0.lock())
    }
}


#[allow(unused_imports)]
use std::{
    collections::{
        HashMap as StdHashMap,
        HashSet as StdHashSet,
    },
    fmt,
    ops::{Deref, DerefMut},
};

#[cfg(feature = "ahash")]
use ahash::RandomState;

#[cfg(not(feature = "ahash"))]
use std::collections::hash_map::RandomState;

pub(crate) struct HashMap<K, V>(StdHashMap<K, V, RandomState>);

impl<K, V> HashMap<K, V> {
    #[inline]
    pub fn new() -> Self {
        Self(StdHashMap::with_hasher(RandomState::new()))
    }
}

impl<K, V> Deref for HashMap<K, V> {
    type Target = StdHashMap<K, V, RandomState>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<K, V> DerefMut for HashMap<K, V> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<K, V> fmt::Debug for HashMap<K, V>
where
    StdHashMap<K, V, RandomState>: fmt::Debug,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(feature = "hot-reloading")]
pub(crate) struct HashSet<T>(StdHashSet<T, RandomState>);

#[cfg(feature = "hot-reloading")]
impl<T> HashSet<T> {
    #[inline]
    pub fn new() -> Self {
        Self(StdHashSet::with_hasher(RandomState::new()))
    }
}

#[cfg(feature = "hot-reloading")]
impl<T> Deref for HashSet<T> {
    type Target = StdHashSet<T, RandomState>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(feature = "hot-reloading")]
impl<T> DerefMut for HashSet<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(feature = "hot-reloading")]
impl<T> fmt::Debug for HashSet<T>
where
    StdHashSet<T, RandomState>: fmt::Debug,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}


#[cfg(feature = "hot-reloading")]
#[derive(Debug)]
pub struct DepsRecord(pub(crate) HashSet<crate::cache::OwnedKey>);
