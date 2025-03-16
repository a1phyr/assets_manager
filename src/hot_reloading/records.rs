use crate::{SharedString, key::AssetKey, utils::HashSet};
use std::{cell::Cell, ptr::NonNull};

use super::HotReloader;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Dependency {
    File(SharedString, SharedString),
    Directory(SharedString),
    Asset(AssetKey),
}

impl Dependency {
    pub fn as_borrowed(&self) -> BorrowedDependency<'_> {
        match self {
            Dependency::File(id, ext) => BorrowedDependency::File(id, ext),
            Dependency::Directory(id) => BorrowedDependency::Directory(id),
            Dependency::Asset(key) => BorrowedDependency::Asset(key),
        }
    }
}

pub(crate) type Dependencies = HashSet<Dependency>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BorrowedDependency<'a> {
    File(&'a SharedString, &'a SharedString),
    Directory(&'a SharedString),
    Asset(&'a AssetKey),
}

impl BorrowedDependency<'_> {
    pub fn into_owned(self) -> Dependency {
        match self {
            BorrowedDependency::File(id, ext) => Dependency::File(id.clone(), ext.clone()),
            BorrowedDependency::Directory(id) => Dependency::Directory(id.clone()),
            BorrowedDependency::Asset(key) => Dependency::Asset(key.clone()),
        }
    }
}

impl hashbrown::Equivalent<Dependency> for BorrowedDependency<'_> {
    fn equivalent(&self, key: &Dependency) -> bool {
        *self == key.as_borrowed()
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
            records: Dependencies::new(),
        }
    }

    fn insert_asset(&mut self, reloader: &HotReloader, key: AssetKey) {
        if self.reloader == reloader {
            self.records.insert(Dependency::Asset(key));
        }
    }

    fn insert_file(&mut self, reloader: &HotReloader, id: SharedString, ext: SharedString) {
        if self.reloader == reloader {
            self.records.insert(Dependency::File(id, ext));
        }
    }

    fn insert_dir(&mut self, reloader: &HotReloader, id: SharedString) {
        if self.reloader == reloader {
            self.records.insert(Dependency::Directory(id));
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
    static RECORDING: Cell<Option<NonNull<Record>>> = const { Cell::new(None) };
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

pub(crate) fn add_record(reloader: &HotReloader, key: AssetKey) {
    RECORDING.with(|rec| {
        if let Some(mut recorder) = rec.get() {
            let recorder = unsafe { recorder.as_mut() };
            recorder.insert_asset(reloader, key);
        }
    });
}

pub(crate) fn add_file_record(reloader: &HotReloader, id: &str, ext: &str) {
    RECORDING.with(|rec| {
        if let Some(mut recorder) = rec.get() {
            let recorder = unsafe { recorder.as_mut() };
            recorder.insert_file(reloader, id.into(), ext.into());
        }
    });
}

pub(crate) fn add_dir_record(reloader: &HotReloader, id: &str) {
    RECORDING.with(|rec| {
        if let Some(mut recorder) = rec.get() {
            let recorder = unsafe { recorder.as_mut() };
            recorder.insert_dir(reloader, id.into());
        }
    });
}
