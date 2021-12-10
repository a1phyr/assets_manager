use std::{
    fmt,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use crate::{
    source::DirEntry,
    utils::{path_of_entry, HashMap},
    BoxedError,
};

#[cfg(doc)]
use crate::source::Source;

pub struct SmallSet<T>(Vec<T>);

impl<T> SmallSet<T> {
    #[inline]
    pub const fn new() -> Self {
        Self(Vec::new())
    }
}

impl<T: Eq> SmallSet<T> {
    #[inline]
    pub fn contains(&mut self, elem: &T) -> bool {
        self.0.iter().any(|x| x == elem)
    }

    #[inline]
    pub fn insert(&mut self, elem: T) {
        if !self.contains(&elem) {
            self.0.push(elem)
        }
    }
}

struct UpdateSender(crossbeam_channel::Sender<super::UpdateMessage>);

impl super::UpdateSender for UpdateSender {
    fn send_update(&self, message: super::UpdateMessage) {
        let _ = self.0.send(message);
    }
}

/// Built-in reloader based on filesystem events.
///
/// You can use it to quickly set up hot-reloading for a custom [`Source`].
pub struct FsWatcherBuilder {
    roots: Vec<PathBuf>,
    watcher: notify::RecommendedWatcher,
    notify: mpsc::Receiver<notify::DebouncedEvent>,
}

impl FsWatcherBuilder {
    /// Creates a new builder.
    pub fn new() -> Result<Self, BoxedError> {
        let (notify_tx, notify) = mpsc::channel();
        let watcher = notify::watcher(notify_tx, Duration::from_millis(50))?;
        Ok(Self {
            roots: Vec::new(),
            watcher,
            notify,
        })
    }

    /// Adds a path to watch.
    pub fn watch(&mut self, path: PathBuf) -> Result<(), BoxedError> {
        notify::Watcher::watch(&mut self.watcher, &path, notify::RecursiveMode::Recursive)?;
        self.roots.push(path);
        Ok(())
    }

    /// Starts the watcher.
    ///
    /// The return value is meant to be used in [`Source::configure_hot_reloading`]
    pub fn build(self, events: super::EventSender) -> super::DynUpdateSender {
        let (sender, updates) = crossbeam_channel::unbounded();

        thread::Builder::new()
            .name("assets_translate".to_string())
            .spawn(|| translation_thread(self.watcher, self.roots, self.notify, updates, events))
            .unwrap();

        Box::new(UpdateSender(sender))
    }
}

impl fmt::Debug for FsWatcherBuilder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("FsWatcherBuilder")
            .field("roots", &self.roots)
            .finish()
    }
}

struct WatchedPaths {
    roots: Vec<PathBuf>,
    paths: HashMap<PathBuf, SmallSet<super::AssetKey>>,
}

impl WatchedPaths {
    fn new(roots: Vec<PathBuf>) -> WatchedPaths {
        WatchedPaths {
            roots,
            paths: HashMap::new(),
        }
    }

    fn add_asset(&mut self, asset: super::AssetKey) {
        for root in &self.roots {
            for ext in asset.typ.extensions() {
                let path = path_of_entry(root, DirEntry::File(&asset.id, ext));
                self.paths
                    .entry(path)
                    .or_insert_with(SmallSet::new)
                    .insert(asset.clone());
            }
        }
    }

    fn remove_asset(&mut self, asset: super::AssetKey) {
        for root in &self.roots {
            for ext in asset.typ.extensions() {
                let path = path_of_entry(root, DirEntry::File(&asset.id, ext));
                self.paths.remove(&path);
            }
        }
    }

    fn clear(&mut self) {
        self.paths.clear();
    }

    fn assets<'a>(&'a self, path: &Path) -> impl Iterator<Item = super::AssetKey> + 'a {
        self.paths
            .get(path)
            .map_or(&[][..], |set| &set.0)
            .iter()
            .cloned()
    }
}

fn translation_thread(
    _watcher: notify::RecommendedWatcher,
    roots: Vec<PathBuf>,
    notify: mpsc::Receiver<notify::DebouncedEvent>,
    updates: crossbeam_channel::Receiver<super::UpdateMessage>,
    events: super::EventSender,
) {
    log::trace!("Starting hot-reloading translation thread");

    let mut watched_paths = WatchedPaths::new(roots);

    while let Ok(event) = notify.recv() {
        loop {
            match updates.try_recv() {
                Ok(super::UpdateMessage::AddAsset(key)) => watched_paths.add_asset(key),
                Ok(super::UpdateMessage::RemoveAsset(key)) => watched_paths.remove_asset(key),
                Ok(super::UpdateMessage::Clear) => watched_paths.clear(),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        }

        log::trace!("Received filesystem event: {:?}", event);

        match event {
            notify::DebouncedEvent::Write(path)
            | notify::DebouncedEvent::Chmod(path)
            | notify::DebouncedEvent::Rename(_, path)
            | notify::DebouncedEvent::Create(path) => {
                for asset in watched_paths.assets(&path) {
                    if events.send(asset).is_err() {
                        return;
                    }
                }
            }
            _ => (),
        }
    }
}
