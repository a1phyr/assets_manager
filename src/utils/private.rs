//! Utilities for the whole crate
//!
//! This module contains:
//! - Keys to represent assets
//! - An unified API for synchronisation primitives between `std` and `parking_lot`
//! - An unified API for `HashMap`s between `std` and `ahash` hashers
//! - A marker for private APIs

use crate::{source::DirEntry, SharedString};

#[allow(unused_imports)]
use std::{
    any::TypeId,
    borrow::Borrow,
    collections::{HashMap as StdHashMap, HashSet as StdHashSet},
    fmt, hash,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

pub fn path_of_entry(root: &Path, entry: DirEntry) -> PathBuf {
    let (id, ext) = match entry {
        DirEntry::File(id, ext) => (id, Some(ext)),
        DirEntry::Directory(id) => (id, None),
    };

    let capacity = root.as_os_str().len() + id.len() + ext.map_or(0, |ext| ext.len()) + 2;
    let mut path = PathBuf::with_capacity(capacity);

    path.push(root);
    path.extend(id.split('.'));
    if let Some(ext) = ext {
        path.set_extension(ext);
    }

    path
}

#[inline]
pub(crate) fn extension_of(path: &Path) -> Option<&str> {
    match path.extension() {
        Some(ext) => ext.to_str(),
        None => Some(""),
    }
}

/// Trick to be able to use a `BorrowedKey` to index a `HashMap<OwnedKey, _>`.
///
/// See https://stackoverflow.com/questions/45786717/how-to-implement-hashmap-with-two-keys/45795699#45795699.
///
/// TODO: Remove this in favor of the `raw_entry` API when it is stabilized.
pub(crate) trait Key {
    fn id(&self) -> &str;
    fn type_id(&self) -> TypeId;
}

impl PartialEq for dyn Key + '_ {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.type_id() == other.type_id() && self.id() == other.id()
    }
}

impl Eq for dyn Key + '_ {}

impl hash::Hash for dyn Key + '_ {
    #[inline]
    fn hash<H: hash::Hasher>(&self, h: &mut H) {
        self.type_id().hash(h);
        self.id().hash(h);
    }
}

/// The key used to identify assets
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct OwnedKey {
    pub type_id: TypeId,
    pub id: SharedString,
}

impl OwnedKey {
    /// Creates a `OwnedKey` with the given type and id.
    #[inline]
    #[allow(dead_code)]
    pub fn new<T: 'static>(id: SharedString) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
        }
    }

    #[inline]
    pub fn new_with(id: SharedString, type_id: TypeId) -> Self {
        Self { id, type_id }
    }

    #[inline]
    pub fn borrow(&self) -> BorrowedKey {
        BorrowedKey {
            id: &self.id,
            type_id: self.type_id,
        }
    }

    #[cfg(feature = "hot-reloading")]
    #[inline]
    pub fn into_id(self) -> SharedString {
        self.id
    }
}

impl Key for OwnedKey {
    fn id(&self) -> &str {
        &self.id
    }

    fn type_id(&self) -> TypeId {
        self.type_id
    }
}

impl<'a> Borrow<dyn Key + 'a> for OwnedKey {
    #[inline]
    fn borrow(&self) -> &(dyn Key + 'a) {
        self
    }
}

impl fmt::Debug for OwnedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.borrow(), f)
    }
}

/// A borrowed version of [`OwnedKey`]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BorrowedKey<'a> {
    type_id: TypeId,
    id: &'a str,
}

impl<'a> BorrowedKey<'a> {
    /// Creates an Key for the given type and id.
    #[inline]
    #[allow(dead_code)]
    pub fn new<T: 'static>(id: &'a str) -> Self {
        Self {
            id,
            type_id: TypeId::of::<T>(),
        }
    }

    #[inline]
    pub fn new_with(id: &'a str, type_id: TypeId) -> Self {
        Self { id, type_id }
    }

    #[inline]
    pub fn to_owned(self) -> OwnedKey {
        OwnedKey {
            id: self.id.into(),
            type_id: self.type_id,
        }
    }
}

impl Key for BorrowedKey<'_> {
    fn id(&self) -> &str {
        self.id
    }

    fn type_id(&self) -> TypeId {
        self.type_id
    }
}

impl From<BorrowedKey<'_>> for OwnedKey {
    fn from(key: BorrowedKey) -> Self {
        key.to_owned()
    }
}

impl fmt::Debug for BorrowedKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("id", &self.id)
            .field("type_id", &self.type_id)
            .finish()
    }
}

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

#[allow(unused)]
impl<T> RwLock<T> {
    #[inline]
    pub fn new(inner: T) -> Self {
        Self(sync::RwLock::new(inner))
    }

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

#[allow(unused)]
#[derive(Default)]
pub(crate) struct Mutex<T: ?Sized>(sync::Mutex<T>);

#[allow(unused)]
impl<T> Mutex<T> {
    #[inline]
    pub fn new(inner: T) -> Self {
        Self(sync::Mutex::new(inner))
    }
}

#[allow(unused)]
impl<T: ?Sized> Mutex<T> {
    #[inline]
    pub fn lock(&self) -> sync::MutexGuard<T> {
        wrap(self.0.lock())
    }
}

#[allow(unused)]
#[derive(Default)]
pub(crate) struct Condvar(sync::Condvar);

#[allow(unused)]
impl Condvar {
    #[inline]
    pub fn new() -> Self {
        Self(sync::Condvar::new())
    }

    #[inline]
    pub fn notify_all(&self) {
        self.0.notify_all();
    }

    #[inline]
    pub fn wait_while<'a, T, F>(
        &self,
        mut guard: sync::MutexGuard<'a, T>,
        mut condition: F,
    ) -> sync::MutexGuard<'a, T>
    where
        F: FnMut(&mut T) -> bool,
    {
        #[cfg(feature = "parking_lot")]
        {
            while condition(&mut guard) {
                self.0.wait(&mut guard);
            }
            guard
        }

        #[cfg(not(feature = "parking_lot"))]
        {
            while condition(&mut guard) {
                guard = wrap(self.0.wait(guard));
            }
            guard
        }
    }
}

mod private_marker {
    pub trait PrivateMarker {}
    pub(crate) enum Private {}
    impl PrivateMarker for Private {}
}

pub(crate) use private_marker::{Private, PrivateMarker};

#[cfg(feature = "ahash")]
pub(crate) use ahash::RandomState;

#[cfg(not(feature = "ahash"))]
pub(crate) use std::collections::hash_map::RandomState;

pub(crate) struct HashMap<K, V>(StdHashMap<K, V, RandomState>);

impl<K, V> HashMap<K, V> {
    #[inline]
    #[allow(unused)]
    pub fn new() -> Self {
        Self(StdHashMap::with_hasher(RandomState::new()))
    }

    #[inline]
    pub fn with_hasher(hasher: RandomState) -> Self {
        Self(StdHashMap::with_hasher(hasher))
    }

    #[cfg(feature = "zip")]
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self(StdHashMap::with_capacity_and_hasher(
            capacity,
            RandomState::new(),
        ))
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
pub struct DepsRecord(pub(crate) HashSet<OwnedKey>);
