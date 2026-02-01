use crate::{
    key::AssetKey,
    utils::{HashSet, Mutex, SharedString},
};
use std::{cell::Cell, fmt, ptr::NonNull, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Dependency {
    File(SharedString, SharedString),
    Directory(SharedString),
    Asset(AssetKey),
}

pub(crate) type Dependencies = HashSet<Dependency>;

struct Record {
    records: Dependencies,
    additional: Option<Arc<Mutex<Dependencies>>>,
}

impl Record {
    fn install<T>(&mut self, f: impl FnOnce() -> T) -> T {
        RECORDING.with(|rec| {
            let _guard = CellGuard::replace(rec, Some(NonNull::from(self)));
            f()
        })
    }

    fn collect(mut self) -> Dependencies {
        if let Some(more) = self.additional {
            self.records.extend(more.lock().drain());
        }

        self.records
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

pub(crate) fn record<F: FnOnce() -> T, T>(f: F) -> (T, Dependencies) {
    let mut record = Record {
        records: Dependencies::new(),
        additional: None,
    };
    let res = record.install(f);
    (res, record.collect())
}

/// Temporarily prevent `Asset` dependencies to be recorded.
///
/// This function disables dependencies recording in [`Asset::load`]. Assets and
/// files loaded within the closure will not be recorded as dependencies.
/// Therefore, change of one of these dependencies will not trigger a reload of
/// the asset.
///
/// When hot-reloading is disabled, this function simply returns the result of
/// the closure given as parameter.
#[inline]
pub fn no_record<T>(f: impl FnOnce() -> T) -> T {
    RECORDING.with(|rec| {
        let _guard = CellGuard::replace(rec, None);
        f()
    })
}

pub(crate) fn add_asset(make_key: impl FnOnce() -> AssetKey) {
    RECORDING.with(|rec| {
        if let Some(mut recorder) = rec.get() {
            let recorder = unsafe { recorder.as_mut() };

            recorder.records.insert(Dependency::Asset(make_key()));
        }
    });
}

pub(crate) fn add_file_record(id: &str, ext: &str) {
    RECORDING.with(|rec| {
        if let Some(mut recorder) = rec.get() {
            let recorder = unsafe { recorder.as_mut() };

            recorder
                .records
                .insert(Dependency::File(id.into(), ext.into()));
        }
    });
}

pub(crate) fn add_dir_record(id: &str) {
    RECORDING.with(|rec| {
        if let Some(mut recorder) = rec.get() {
            let recorder = unsafe { recorder.as_mut() };

            recorder.records.insert(Dependency::Directory(id.into()));
        }
    });
}

/// Records dependencies for hot-reloading.
///
/// This type is only useful if you do multi-threading within asset loading
/// (e.g. if you use `rayon` in `Asset::load`).
#[derive(Clone)]
pub struct Recorder(Option<Arc<Mutex<Dependencies>>>);

impl Recorder {
    /// Gets the recorder which is currently installed.
    ///
    /// If no recorder is currently install, this returns a recorder that does
    /// nothing when installed.
    pub fn current() -> Self {
        let deps = RECORDING.with(|rec| {
            let mut rec = rec.get()?;
            let recorder = unsafe { rec.as_mut() };
            let deps = recorder.additional.get_or_insert_default().clone();
            Some(deps)
        });
        Self(deps)
    }

    /// Runs the given closure with the recorder installed.
    pub fn install<T>(&self, f: impl FnOnce() -> T) -> T {
        match &self.0 {
            Some(deps) => {
                let mut record = Record {
                    records: Dependencies::new(),
                    additional: Some(deps.clone()),
                };
                let res = record.install(f);
                deps.lock().extend(record.records);
                res
            }
            None => f(),
        }
    }
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Recorder { .. }")
    }
}
