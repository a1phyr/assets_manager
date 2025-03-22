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
use std::thread;

use crate::{
    SharedString,
    cache::WeakAssetCache,
    key::{AssetKey, Type},
    source::{OwnedDirEntry, Source},
    utils::HashSet,
};

pub use records::Recorder;
pub use watcher::FsWatcherBuilder;

pub(crate) use records::{BorrowedDependency, Dependencies, Dependency};

enum CacheMessage {
    AddAsset(AssetKey, Dependencies),
}

/// An error returned when an end of a channel was disconnected.
#[derive(Debug)]
pub struct Disconnected;

enum Events {
    Single(OwnedDirEntry),
    Multiple(Vec<OwnedDirEntry>),
}

impl Events {
    fn for_each(self, mut f: impl FnMut(OwnedDirEntry)) {
        match self {
            Self::Single(e) => f(e),
            Self::Multiple(e) => e.into_iter().for_each(f),
        }
    }
}

/// Sends events for hot-reloading.
#[derive(Debug, Clone)]
pub struct EventSender(Sender<Events>);

impl EventSender {
    /// Sends an event.
    ///
    /// A matching asset in the cache will be reloaded, and with it compounds
    /// that depends on it.
    #[inline]
    pub fn send(&self, event: OwnedDirEntry) -> Result<(), Disconnected> {
        self.0.send(Events::Single(event)).or(Err(Disconnected))
    }

    /// Sends multiple events an once.
    ///
    /// If successful, this function returns the number of events sent.
    pub fn send_multiple<I>(&self, events: I) -> Result<usize, Disconnected>
    where
        I: IntoIterator<Item = OwnedDirEntry>,
    {
        let mut events = events.into_iter();
        let event = match events.size_hint().1 {
            Some(0) => return Ok(0),
            Some(1) => match events.next() {
                Some(event) => Events::Single(event),
                None => return Ok(0),
            },
            _ => Events::Multiple(events.collect()),
        };

        let len = match &event {
            Events::Single(_) => 1,
            Events::Multiple(events) => events.len(),
        };

        match self.0.send(event) {
            Ok(()) => Ok(len),
            Err(_) => Err(Disconnected),
        }
    }
}

/// The hot-reloading handler.
pub(crate) struct HotReloader {
    sender: Sender<CacheMessage>,
}

impl HotReloader {
    /// Starts hot-reloading.
    pub fn start(cache: WeakAssetCache, source: &dyn Source) -> Option<Self> {
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
            .spawn(|| hot_reloading_thread(events_rx, cache_msg_rx, cache))
            .map_err(|err| log::error!("Failed to start hot-reloading thread: {err}"))
            .ok()?;

        Some(Self {
            sender: cache_msg_tx,
        })
    }

    fn addr(&self) -> usize {
        (self as *const Self).addr()
    }

    // All theses methods ignore send/recv errors: the program can continue
    // without hot-reloading if it stopped, and an error should have already
    // been logged.

    pub(crate) fn add_asset(&self, id: SharedString, deps: Dependencies, typ: Type) {
        let key = AssetKey::new(id, typ);
        let _ = self.sender.send(CacheMessage::AddAsset(key, deps));
    }
}

fn hot_reloading_thread(
    events: Receiver<Events>,
    cache_msg: Receiver<CacheMessage>,
    asset_cache: WeakAssetCache,
) {
    log::info!("Starting hot-reloading");

    let mut cache = HotReloadingData::new(asset_cache);

    let mut select = channel::Select::new();
    select.recv(&cache_msg);
    select.recv(&events);

    loop {
        // We don't use `select` method here as we always want to check
        // `cache_msg` channel first.
        let ready = select.ready();

        loop {
            match cache_msg.try_recv() {
                Ok(CacheMessage::AddAsset(key, deps)) => cache.add_asset(key, deps),
                Err(channel::TryRecvError::Empty) => break,
                Err(channel::TryRecvError::Disconnected) => return,
            }
        }

        if ready == 1 {
            match events.try_recv() {
                Ok(msg) => cache.handle_events(msg),
                Err(channel::TryRecvError::Empty) => (),
                // We won't receive events anymore, we can stop now
                Err(channel::TryRecvError::Disconnected) => break,
            }
        }
    }

    log::info!("Stopping hot-reloading");
}

struct HotReloadingData {
    // It is important to keep a weak reference here, because we rely on the
    // fact that dropping the `HotReloader` drop the channel and therefore stop
    // the hot-reloading thread
    cache: WeakAssetCache,
    to_reload: HashSet<OwnedDirEntry>,
    deps: dependencies::DepsGraph,
}

impl HotReloadingData {
    fn new(cache: WeakAssetCache) -> Self {
        HotReloadingData {
            to_reload: HashSet::new(),
            cache,
            deps: dependencies::DepsGraph::new(),
        }
    }

    fn handle_events(&mut self, events: Events) {
        events.for_each(|entry| {
            if self.deps.contains(&entry) {
                log::trace!("New event: {entry:?}");
                self.to_reload.insert(entry);
            }
        });
        self.run_update();
    }

    fn run_update(&mut self) {
        if let Some(asset_cache) = &mut self.cache.upgrade() {
            let to_update = self.deps.topological_sort_from(self.to_reload.iter());
            self.to_reload.clear();

            for key in to_update.into_iter() {
                let new_deps = asset_cache.reload_untyped(&key.id, key.typ);

                if let Some(new_deps) = new_deps {
                    self.deps.insert_asset(key, new_deps);
                };
            }
        }
    }

    fn add_asset(&mut self, key: AssetKey, deps: Dependencies) {
        self.deps.insert_asset(key, deps);
    }
}
