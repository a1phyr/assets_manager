use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use crossbeam_channel::{self as channel, Receiver, Sender};

use crate::{
    source::DirEntry,
    utils::{path_of_entry, HashMap},
};

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

use super::{AssetKey, UpdateMessage};

pub(super) fn make(
    root: PathBuf,
    updates: Receiver<UpdateMessage>,
) -> notify::Result<(notify::RecommendedWatcher, Receiver<AssetKey>)> {
    let (notify_tx, notify_rx) = mpsc::channel();
    let (events_tx, events_rx) = channel::unbounded();

    let mut watcher = notify::watcher(notify_tx, Duration::from_millis(50))?;
    notify::Watcher::watch(&mut watcher, &root, notify::RecursiveMode::Recursive)?;

    thread::Builder::new()
        .name("assets_translate".to_string())
        .spawn(|| translation_thread(root, notify_rx, updates, events_tx))
        .unwrap();

    Ok((watcher, events_rx))
}

struct WatchedPaths {
    root: PathBuf,
    paths: HashMap<PathBuf, SmallSet<AssetKey>>,
}

impl WatchedPaths {
    fn new(root: PathBuf) -> WatchedPaths {
        WatchedPaths {
            root,
            paths: HashMap::new(),
        }
    }

    fn add_asset(&mut self, asset: AssetKey) {
        for ext in asset.typ.extensions() {
            let path = path_of_entry(&self.root, DirEntry::File(&asset.id, ext));
            self.paths
                .entry(path)
                .or_insert_with(SmallSet::new)
                .insert(asset.clone());
        }
    }

    fn clear(&mut self) {
        self.paths.clear();
    }

    fn assets<'a>(&'a self, path: &Path) -> impl Iterator<Item = AssetKey> + 'a {
        self.paths
            .get(path)
            .map_or(&[][..], |set| &set.0)
            .iter()
            .cloned()
    }
}

fn translation_thread(
    root: PathBuf,
    notify: mpsc::Receiver<notify::DebouncedEvent>,
    updates: Receiver<UpdateMessage>,
    events: Sender<AssetKey>,
) {
    let mut watched_paths = WatchedPaths::new(root);

    while let Ok(event) = notify.recv() {
        loop {
            match updates.try_recv() {
                Ok(UpdateMessage::AddAsset(key)) => watched_paths.add_asset(key),
                Ok(UpdateMessage::Clear) => watched_paths.clear(),
                Err(channel::TryRecvError::Empty) => break,
                Err(channel::TryRecvError::Disconnected) => return,
            }
        }

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
