//! Tools to work with hot-reloading.

mod dependencies;
mod paths;
mod watcher;

#[cfg(test)]
mod tests;

use paths::{CompoundReloadInfos, HotReloadingData};

use crossbeam_channel::{self as channel, Receiver, Sender};
use std::{fmt, ptr::NonNull, thread};

use crate::{
    source::Source,
    utils::{HashSet, OwnedKey},
    AssetCache, SharedString,
};

pub use crate::key::{AssetKey, AssetType};
pub use watcher::FsWatcherBuilder;

type DynAssetCache = crate::AssetCache<dyn crate::source::Source>;

/// A message with an update of the state of the [`AssetCache`].
#[non_exhaustive]
#[derive(Debug)]
pub enum UpdateMessage {
    /// An asset was added to the cache.
    AddAsset(AssetKey),

    /// The cache was cleared.
    Clear,
}

enum CacheMessage {
    Ptr(NonNull<AssetCache<dyn Source + Sync>>),
    Static(&'static AssetCache<dyn Source + Sync>),

    Clear,
    AddCompound(CompoundReloadInfos),
}
unsafe impl Send for CacheMessage where AssetCache<dyn Source + Sync>: Sync {}

/// An error returned when an end of a channel was disconnected.
#[derive(Debug)]
pub struct Disconnected;

/// Sends events for hot-reloading.
#[derive(Debug, Clone)]
pub struct EventSender(Sender<AssetKey>);

impl EventSender {
    /// Sends an event.
    ///
    /// A matching asset in the cache will be reloaded, and with it compounds
    /// that depends on it.
    #[inline]
    pub fn send(&self, event: AssetKey) -> Result<(), Disconnected> {
        self.0.send(event).or(Err(Disconnected))
    }
}

/// An error returned by [`UpdateReceiver::try_recv`].
#[derive(Debug)]
pub enum TryRecvUpdateError {
    /// The channel was disconnected.
    Disconnected,

    /// The channel is empty.
    Empty,
}

/// Receives updates about the state of an `AssetCache`.
#[derive(Debug)]
pub struct UpdateReceiver(Receiver<UpdateMessage>);

impl UpdateReceiver {
    /// Blocks until an update is received.
    #[inline]
    pub fn recv(&self) -> Result<UpdateMessage, Disconnected> {
        self.0.recv().or(Err(Disconnected))
    }

    /// Attepts to receive an update without waiting.
    #[inline]
    pub fn try_recv(&self) -> Result<UpdateMessage, TryRecvUpdateError> {
        self.0.try_recv().map_err(|err| match err {
            channel::TryRecvError::Disconnected => TryRecvUpdateError::Disconnected,
            channel::TryRecvError::Empty => TryRecvUpdateError::Empty,
        })
    }
}

/// Configuration for hot-reloading.
///
/// It can be created with [`config_hot_reloading`] and is meant to be used in
/// [`HotReloader::start`].
#[derive(Debug)]
pub struct HotReloaderConfig {
    updates: Sender<UpdateMessage>,
    events: Receiver<AssetKey>,
}

/// Creates the necessary parts to hook hot-reloading.
#[inline]
pub fn config_hot_reloading() -> (EventSender, UpdateReceiver, HotReloaderConfig) {
    let (events_tx, events) = channel::unbounded();
    let (updates, updates_rx) = channel::unbounded();
    (
        EventSender(events_tx),
        UpdateReceiver(updates_rx),
        HotReloaderConfig { events, updates },
    )
}

/// The hot-reloading handler.
pub struct HotReloader {
    sender: Sender<CacheMessage>,
    receiver: Receiver<()>,
    updates: Sender<UpdateMessage>,
}

impl HotReloader {
    /// Starts hot-reloading.
    pub fn start<S: Source + Send + 'static>(config: HotReloaderConfig, source: S) -> Self {
        let (cache_msg_tx, cache_msg_rx) = channel::unbounded();
        let (answer_tx, answer_rx) = channel::unbounded();
        let events = config.events;

        thread::Builder::new()
            .name("assets_hot_reload".to_string())
            .spawn(|| Self::hot_reloading_thread(source, events, cache_msg_rx, answer_tx))
            .unwrap();

        HotReloader {
            updates: config.updates,
            sender: cache_msg_tx,
            receiver: answer_rx,
        }
    }

    // this is done in a new thread
    fn hot_reloading_thread<S: Source>(
        source: S,
        events: Receiver<AssetKey>,
        cache_msg: Receiver<CacheMessage>,
        answer: Sender<()>,
    ) {
        log::info!("Starting hot-reloading");

        let mut cache = HotReloadingData::new(source);

        // At the beginning, we select over three channels:
        // - One to notify that we can update the `AssetCache` or that we
        //   can switch to using a 'static reference. We close this channel
        //   in the latter case.
        // - One to receive events from notify
        // - One to update the watched paths list when the `AssetCache`
        //   changes
        let mut select = channel::Select::new();
        select.recv(&cache_msg);
        select.recv(&events);

        loop {
            let ready = select.select();
            match ready.index() {
                0 => match ready.recv(&cache_msg) {
                    Ok(CacheMessage::Ptr(ptr)) => {
                        // Safety: The received pointer is guaranteed to
                        // be valid until we reply back
                        cache.update_if_local(unsafe { ptr.as_ref() });
                        answer.send(()).unwrap();
                    }
                    Ok(CacheMessage::Static(asset_cache)) => cache.use_static_ref(asset_cache),
                    Ok(CacheMessage::Clear) => cache.clear_local_cache(),
                    Ok(CacheMessage::AddCompound(infos)) => cache.add_compound(infos),
                    Err(_) => (),
                },

                1 => match ready.recv(&events) {
                    Ok(msg) => cache.load_asset(msg),
                    Err(_) => {
                        log::error!("Notify panicked, hot-reloading stopped");
                        break;
                    }
                },

                _ => unreachable!(),
            }
        }
    }

    // All theses methods ignore send/recv errors: the program can continue
    // without hot-reloading if it stopped, and an error should have already
    // been logged.

    pub(crate) fn add_asset<A: crate::Asset>(&self, id: SharedString) {
        let _ = self
            .updates
            .send(UpdateMessage::AddAsset(AssetKey::new::<A>(id)));
    }

    pub(crate) fn add_compound<A: crate::Compound>(
        &self,
        id: SharedString,
        deps: HashSet<OwnedKey>,
    ) {
        let infos = CompoundReloadInfos::of::<A>(id, deps);
        let _ = self.sender.send(CacheMessage::AddCompound(infos));
    }

    pub(crate) fn clear(&self) {
        let _ = self.sender.send(CacheMessage::Clear);
        let _ = self.updates.send(UpdateMessage::Clear);
    }

    pub(crate) fn reload(&self, cache: &AssetCache<dyn Source + Sync + '_>) {
        // Lifetime magic
        let ptr = unsafe { std::mem::transmute(NonNull::from(cache)) };
        let _ = self.sender.send(CacheMessage::Ptr(ptr));
        let _ = self.receiver.recv();
    }

    pub(crate) fn send_static(&self, cache: &'static AssetCache<dyn Source + Sync>) {
        let _ = self.sender.send(CacheMessage::Static(cache));
    }
}

impl fmt::Debug for HotReloader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("HotReloader { .. }")
    }
}
