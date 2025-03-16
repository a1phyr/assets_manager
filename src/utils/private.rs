//! Utilities for the whole crate
//!
//! This module contains:
//! - Keys to represent assets
//! - An unified API for synchronisation primitives between `std` and `parking_lot`
//! - An unified API for `HashMap`s between `std` and `ahash` hashers
//! - A marker for private APIs

#[allow(unused_imports)]
use crate::{SharedString, source::DirEntry};

use std::{
    fmt,
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

/// Build ids from components.
///
/// Using this allows to easily reuse buffers when building several ids in a
/// row, and thus to avoid repeated allocations.
#[cfg(any(feature = "tar", feature = "zip", feature = "hot-reloading"))]
#[derive(Default)]
pub struct IdBuilder {
    buf: String,
}

#[cfg(any(feature = "tar", feature = "zip", feature = "hot-reloading"))]
impl IdBuilder {
    /// Pushs a segment in the builder.
    pub fn push(&mut self, s: &str) -> Option<()> {
        if s.contains('.') {
            return None;
        }

        if !self.buf.is_empty() {
            self.buf.push('.');
        }
        self.buf.push_str(s);
        Some(())
    }

    /// Pops a segment from the builder.
    ///
    /// Returns `None` if the builder was empty.
    pub fn pop(&mut self) -> Option<()> {
        if self.buf.is_empty() {
            return None;
        }
        let pos = self.buf.rfind('.').unwrap_or(0);
        self.buf.truncate(pos);
        Some(())
    }

    /// Joins segments to build a id.
    #[inline]
    pub fn join(&self) -> SharedString {
        self.buf.as_str().into()
    }

    /// Resets the builder without freeing buffers.
    #[inline]
    pub fn reset(&mut self) {
        self.buf.clear()
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

/// Fake public structure for internal APIs
#[derive(Debug)]
pub struct Private;

#[cfg(feature = "faster-hash")]
pub(crate) use foldhash::fast::RandomState;

#[cfg(not(feature = "faster-hash"))]
pub(crate) use std::collections::hash_map::RandomState;

pub(crate) struct HashMap<K, V>(hashbrown::HashMap<K, V, RandomState>);

impl<K, V> HashMap<K, V> {
    #[inline]
    #[allow(unused)]
    pub fn new() -> Self {
        Self(hashbrown::HashMap::with_hasher(RandomState::default()))
    }

    #[cfg(feature = "zip")]
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self(hashbrown::HashMap::with_capacity_and_hasher(
            capacity,
            RandomState::default(),
        ))
    }
}

impl<K, V> Deref for HashMap<K, V> {
    type Target = hashbrown::HashMap<K, V, RandomState>;

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
    hashbrown::HashMap<K, V, RandomState>: fmt::Debug,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(feature = "hot-reloading")]
pub(crate) struct HashSet<T>(hashbrown::HashSet<T, RandomState>);

#[cfg(feature = "hot-reloading")]
impl<T> HashSet<T> {
    #[inline]
    pub fn new() -> Self {
        Self(hashbrown::HashSet::with_hasher(RandomState::default()))
    }
}

#[cfg(feature = "hot-reloading")]
impl<T> Deref for HashSet<T> {
    type Target = hashbrown::HashSet<T, RandomState>;

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
    hashbrown::HashSet<T, RandomState>: fmt::Debug,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
