//! Tools to implement hot-reloading.
//!
//! If you don't implement hot-reloading for a custom source, you should not
//! need this.

mod dependencies;
pub(crate) mod records;
mod watcher;

#[cfg(test)]
mod tests;

use crossbeam_channel::{self as channel, Receiver, Sender};
use std::{thread, time};

use crate::{
    cache::{CacheId, WeakAssetCache},
    source::{OwnedDirEntry, Source},
    utils::HashSet,
};

pub use records::Recorder;
pub use watcher::FsWatcherBuilder;

pub(crate) use crate::key::AssetKey;
pub(crate) use records::{Dependencies, Dependency};

enum CacheMessage {
    AddCache(WeakAssetCache),
    RemoveCache(CacheId),
    AddAsset(AssetKey, Dependencies),
}

/// An error returned when an end of a channel was disconnected.
#[derive(Debug)]
pub struct Disconnected;

/// Sends events for hot-reloading.
#[derive(Debug, Clone)]
pub struct EventSender(Sender<Vec<OwnedDirEntry>>);

impl EventSender {
    #[inline]
    pub(crate) fn is_disconnected(&self) -> bool {
        self.0.send(Vec::new()).is_err()
    }

    /// Sends an event.
    ///
    /// A matching asset in the cache will be reloaded, and with it compounds
    /// that depends on it.
    #[inline]
    pub fn send(&self, event: OwnedDirEntry) -> Result<(), Disconnected> {
        self.0.send(vec![event]).or(Err(Disconnected))
    }

    /// Sends multiple events an once.
    ///
    /// If successful, this function returns the number of events sent.
    pub fn send_multiple<I>(&self, events: I) -> Result<usize, Disconnected>
    where
        I: IntoIterator<Item = OwnedDirEntry>,
    {
        let events: Vec<_> = events.into_iter().collect();
        let len = events.len();

        match self.0.send(events) {
            Ok(()) => Ok(len),
            Err(_) => Err(Disconnected),
        }
    }
}

/// The hot-reloading handler.
#[derive(Clone)]
pub(crate) struct HotReloader {
    sender: Sender<CacheMessage>,
}

impl HotReloader {
    /// Starts hot-reloading.
    pub fn start(source: &dyn Source) -> Option<Self> {
        let (events_tx, events_rx) = channel::unbounded();

        if let Err(err) = source.configure_hot_reloading(EventSender(events_tx)) {
            log::error!("Failed to start hot-reloading: {err}");
        }

        // We do a `try_recv` here as a workaround for the lack of method to
        // knwo whether a channel is disconnected. We might lose an event there,
        // but this is fine because there is nothing to reload yet.
        if let Err(channel::TryRecvError::Disconnected) = events_rx.try_recv() {
            // Hot-reloading is unsupported or failed to start
            return None;
        }

        let (cache_msg_tx, cache_msg_rx) = channel::unbounded();

        thread::Builder::new()
            .name("assets_hot_reload".to_string())
            .spawn(|| hot_reloading_thread(events_rx, cache_msg_rx))
            .map_err(|err| log::error!("Failed to start hot-reloading thread: {err}"))
            .ok()?;

        Some(Self {
            sender: cache_msg_tx,
        })
    }

    // All theses methods ignore send/recv errors: the program can continue
    // without hot-reloading if it stopped, and an error should have already
    // been logged.

    pub(crate) fn add_cache(&self, cache: WeakAssetCache) {
        let _ = self.sender.send(CacheMessage::AddCache(cache));
    }

    pub(crate) fn remove_cache(&self, cache: CacheId) {
        let _ = self.sender.send(CacheMessage::RemoveCache(cache));
    }

    pub(crate) fn add_asset(&self, key: AssetKey, deps: Dependencies) {
        let _ = self.sender.send(CacheMessage::AddAsset(key, deps));
    }
}

fn hot_reloading_thread(events: Receiver<Vec<OwnedDirEntry>>, cache_msg: Receiver<CacheMessage>) {
    log::info!("Starting hot-reloading");

    let mut data = HotReloadingData::new();

    let mut select = channel::Select::new_biased();
    select.recv(&cache_msg);
    select.recv(&events);

    // Use a 20ms debouncing time to group reload events and avoid duplicated
    let mut deadline = None;

    loop {
        let ready = match deadline {
            Some(deadline) => select.select_deadline(deadline),
            None => Ok(select.select()),
        };

        // If we reached the deadline, run the update and wait for new events
        let Ok(ready) = ready else {
            deadline = None;
            data.run_update();
            continue;
        };

        match ready.index() {
            0 => match ready.recv(&cache_msg) {
                Ok(CacheMessage::AddCache(weak_cache)) => data.add_cache(weak_cache),
                Ok(CacheMessage::AddAsset(key, deps)) => data.add_asset(key, deps),
                Ok(CacheMessage::RemoveCache(id)) => data.remove_cache(id),
                // There is no more cache to update
                Err(channel::RecvError) => return,
            },

            1 => match ready.recv(&events) {
                Ok(msg) => {
                    let had_events = data.handle_events(msg);

                    // If we don't have a deadline yet, set one 20ms in the future
                    // We don't touch it if we already have one to avoid a continous
                    // event stream preventing running updates.
                    if had_events && deadline.is_none() {
                        deadline = Some(time::Instant::now() + time::Duration::from_millis(20));
                    }
                }
                // We won't receive events anymore, we can stop now
                Err(channel::RecvError) => break,
            },

            _ => unreachable!(),
        }
    }

    log::info!("Stopping hot-reloading");
}

struct HotReloadingData {
    // It is important to keep a weak reference here, because we rely on the
    // fact that dropping the `HotReloader` drop the channel and therefore stop
    // the hot-reloading thread
    caches: HashSet<WeakAssetCache>,
    to_reload: HashSet<Dependency>,
    deps: dependencies::DepsGraph,
}

impl HotReloadingData {
    fn new() -> Self {
        HotReloadingData {
            to_reload: HashSet::new(),
            caches: HashSet::new(),
            deps: dependencies::DepsGraph::new(),
        }
    }

    fn handle_events(&mut self, events: Vec<OwnedDirEntry>) -> bool {
        let mut has_events = false;
        for event in events {
            let entry = event.into_dependency();
            if self.deps.contains(&entry) {
                log::trace!("New event: {entry:?}");
                has_events = true;
                self.to_reload.insert(entry);
            }
        }
        has_events
    }

    fn run_update(&mut self) {
        let to_update = self.deps.topological_sort_from(self.to_reload.iter());
        self.to_reload.clear();

        for key in to_update.into_iter() {
            let Some(weak) = self.caches.get(&key.cache) else {
                continue;
            };

            let Some(asset_cache) = weak.upgrade() else {
                continue;
            };

            let new_deps = asset_cache.reload_untyped(&key);

            if let Some(new_deps) = new_deps {
                self.deps.insert_asset(key, new_deps);
            };
        }
    }

    fn add_cache(&mut self, cache: WeakAssetCache) {
        self.caches.insert(cache);
    }

    fn remove_cache(&mut self, id: CacheId) {
        self.caches.remove(&id);
        self.deps.remove_cache(id);
    }

    fn add_asset(&mut self, key: AssetKey, deps: Dependencies) {
        self.deps.insert_asset(key, deps);
    }
}
