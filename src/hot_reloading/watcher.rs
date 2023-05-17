use std::{
    fmt,
    path::{Path, PathBuf},
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

struct UpdateSender {
    sender: crossbeam_channel::Sender<super::UpdateMessage>,

    // Make sure to keep this alive
    _watcher: notify::RecommendedWatcher,
}

impl super::UpdateSender for UpdateSender {
    fn send_update(&self, message: super::UpdateMessage) {
        let _ = self.sender.send(message);
    }
}

/// Built-in reloader based on filesystem events.
///
/// You can use it to quickly set up hot-reloading for a custom [`Source`].
pub struct FsWatcherBuilder {
    roots: Vec<PathBuf>,
    watcher: notify::RecommendedWatcher,
    payload_sender: crossbeam_channel::Sender<NotifyEventHandler>,
}

impl FsWatcherBuilder {
    /// Creates a new builder.
    pub fn new() -> Result<Self, BoxedError> {
        let (payload_sender, payload_receiver) = crossbeam_channel::unbounded();
        let watcher = notify::recommended_watcher(EventHandlerPayload::new(payload_receiver))?;

        Ok(Self {
            roots: Vec::new(),
            watcher,
            payload_sender,
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

        let watched_paths = WatchedPaths::new(self.roots);
        let event_handler = NotifyEventHandler {
            watched_paths,
            updates,
            events,
        };

        if self.payload_sender.send(event_handler).is_ok() {
            Box::new(UpdateSender {
                sender,
                _watcher: self.watcher,
            })
        } else {
            Box::new(super::UpdateSink)
        }
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

enum EventHandlerPayload<H> {
    Waiting(crossbeam_channel::Receiver<H>),
    Handler(H),
}

impl<H: notify::EventHandler> EventHandlerPayload<H> {
    fn new(receiver: crossbeam_channel::Receiver<H>) -> Self {
        Self::Waiting(receiver)
    }
}

impl<H: notify::EventHandler> notify::EventHandler for EventHandlerPayload<H> {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        match self {
            Self::Waiting(recv) => {
                if let Ok(mut handler) = recv.try_recv() {
                    handler.handle_event(event);
                    *self = Self::Handler(handler);
                }
            }
            Self::Handler(handler) => handler.handle_event(event),
        }
    }
}

struct NotifyEventHandler {
    watched_paths: WatchedPaths,
    updates: crossbeam_channel::Receiver<super::UpdateMessage>,
    events: super::EventSender,
}

impl notify::EventHandler for NotifyEventHandler {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        loop {
            match self.updates.try_recv() {
                Ok(super::UpdateMessage::AddAsset(key)) => self.watched_paths.add_asset(key),
                Ok(super::UpdateMessage::RemoveAsset(key)) => self.watched_paths.remove_asset(key),
                Ok(super::UpdateMessage::Clear) => self.watched_paths.clear(),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        }

        match event {
            Ok(event) => {
                log::trace!("Received filesystem event: {event:?}");

                if matches!(
                    event.kind,
                    notify::EventKind::Any
                        | notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                ) {
                    for path in event.paths {
                        for asset in self.watched_paths.assets(&path) {
                            if self.events.send(asset).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            Err(err) => log::warn!("Error from notify: {err}"),
        }
    }
}
