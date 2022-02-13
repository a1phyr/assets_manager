use crate::utils::{HashSet, OwnedKey};
use std::{cell::Cell, ptr::NonNull};

struct Record {
    id: usize,
    records: HashSet<OwnedKey>,
}

impl Record {
    fn new(id: usize) -> Record {
        Record {
            id,
            records: HashSet::new(),
        }
    }

    fn insert(&mut self, id: usize, key: OwnedKey) {
        if self.id == id {
            self.records.insert(key);
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

pub(crate) fn record<F: FnOnce() -> T, T>(id: usize, f: F) -> (T, HashSet<OwnedKey>) {
    RECORDING.with(|rec| {
        let mut record = Record::new(id);
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

pub(crate) fn add_record(id: usize, key: OwnedKey) {
    RECORDING.with(|rec| {
        if let Some(mut recorder) = rec.get() {
            let recorder = unsafe { recorder.as_mut() };
            recorder.insert(id, key);
        }
    });
}
