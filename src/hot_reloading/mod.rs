//! Tools to implement hot-reloading.
//!
//! If you don't implement hot-reloading for a custom source, you should not
//! need this.

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

/// Defines how to handle updates.
///
/// Cache updates are sent to the hot-reloading subsystem through this trait.
pub trait UpdateSender {
    /// Sends an update to the hot-reloading subsystem. This function should be
    /// quick and should not block.
    fn send_update(&self, message: UpdateMessage);
}

/// A type-erased `UpdateSender`.
pub type DynUpdateSender = Box<dyn UpdateSender + Send + Sync>;

impl<T> UpdateSender for Box<T>
where
    T: UpdateSender + ?Sized,
{
    fn send_update(&self, message: UpdateMessage) {
        (**self).send_update(message)
    }
}

impl<T> UpdateSender for std::sync::Arc<T>
where
    T: UpdateSender + ?Sized,
{
    fn send_update(&self, message: UpdateMessage) {
        (**self).send_update(message)
    }
}

/// The hot-reloading handler.
pub(crate) struct HotReloader {
    sender: Sender<CacheMessage>,
    receiver: Receiver<()>,
    updates: DynUpdateSender,
}

impl HotReloader {
    /// Starts hot-reloading.
    fn start(
        events: Receiver<AssetKey>,
        updates: DynUpdateSender,
        source: Box<dyn Source + Send>,
    ) -> Self {
        let (cache_msg_tx, cache_msg_rx) = channel::unbounded();
        let (answer_tx, answer_rx) = channel::unbounded();

        thread::Builder::new()
            .name("assets_hot_reload".to_string())
            .spawn(|| hot_reloading_thread(source, events, cache_msg_rx, answer_tx))
            .unwrap();

        Self {
            updates,
            sender: cache_msg_tx,
            receiver: answer_rx,
        }
    }

    pub fn make<S: Source>(source: S) -> Option<Self> {
        let sent_source = source.make_source()?;
        let (events_tx, events_rx) = channel::unbounded();

        let updates = source
            .configure_hot_reloading(EventSender(events_tx))
            .map_err(|err| {
                log::error!("Unable to start hot-reloading: {}", err);
            })
            .ok()?;

        Some(Self::start(events_rx, updates, sent_source))
    }

    // All theses methods ignore send/recv errors: the program can continue
    // without hot-reloading if it stopped, and an error should have already
    // been logged.

    pub(crate) fn add_asset<A: crate::Asset>(&self, id: SharedString) {
        let _ = self
            .updates
            .send_update(UpdateMessage::AddAsset(AssetKey::new::<A>(id)));
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
        let _ = self.updates.send_update(UpdateMessage::Clear);
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

fn hot_reloading_thread(
    source: Box<dyn Source>,
    events: Receiver<AssetKey>,
    cache_msg: Receiver<CacheMessage>,
    answer: Sender<()>,
) {
    log::info!("Starting hot-reloading");

    let mut cache = HotReloadingData::new(source);

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
