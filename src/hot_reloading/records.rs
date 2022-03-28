use crate::utils::{HashSet, OwnedKey, SharedString};
use std::{any::TypeId, cell::Cell, ptr::NonNull};

use super::HotReloader;

pub(crate) struct Dependencies(HashSet<OwnedKey>);

impl Dependencies {
    #[inline]
    pub fn empty() -> Self {
        Self(HashSet::new())
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &OwnedKey> {
        self.0.iter()
    }

    #[inline]
    pub fn difference<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = &OwnedKey> + 'a {
        self.0.difference(&other.0)
    }
}

struct Record {
    reloader: *const HotReloader,
    records: Dependencies,
}

impl Record {
    fn new(reloader: &HotReloader) -> Record {
        Record {
            reloader,
            records: Dependencies::empty(),
        }
    }

    fn insert(&mut self, reloader: &HotReloader, key: OwnedKey) {
        if self.reloader == reloader {
            self.records.0.insert(key);
        }
    }
}

/// Makes sure the value in a cell is reset when scope ends.
struct CellGuard<'a, T: Copy> {
    cell: &'a Cell<T>,
    val: T,
}

impl<'a, T: Copy> CellGuard<'a, T> {
    fn replace(cell: &'a Cell<T>, new_value: T) -> Self {
        let val = cell.replace(new_value);
        Self { cell, val }
    }
}

impl<T: Copy> Drop for CellGuard<'_, T> {
    fn drop(&mut self) {
        self.cell.set(self.val);
    }
}

thread_local! {
    static RECORDING: Cell<Option<NonNull<Record>>> = Cell::new(None);
}

pub(crate) fn record<F: FnOnce() -> T, T>(reloader: &HotReloader, f: F) -> (T, Dependencies) {
    RECORDING.with(|rec| {
        let mut record = Record::new(reloader);
        let _guard = CellGuard::replace(rec, Some(NonNull::from(&mut record)));
        let result = f();
        (result, record.records)
    })
}

pub(crate) fn no_record<F: FnOnce() -> T, T>(f: F) -> T {
    RECORDING.with(|rec| {
        let _guard = CellGuard::replace(rec, None);
        f()
    })
}

pub(crate) fn add_record(reloader: &HotReloader, id: SharedString, type_id: TypeId) {
    RECORDING.with(|rec| {
        if let Some(mut recorder) = rec.get() {
            let recorder = unsafe { recorder.as_mut() };
            recorder.insert(reloader, OwnedKey::new_with(id, type_id));
        }
    });
}
