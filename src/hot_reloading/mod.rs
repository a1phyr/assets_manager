//! Tools to implement hot-reloading.
//!
//! If you don't implement hot-reloading for a custom source, you should not
//! need this.

mod dependencies;
mod paths;
pub(crate) mod records;
mod watcher;

#[cfg(test)]
mod tests;

use paths::{CompoundReloadInfos, HotReloadingData};

use crossbeam_channel::{self as channel, Receiver, Sender};
use std::{
    fmt,
    ptr::NonNull,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
};

use crate::{
    asset::Storable,
    key::Type,
    source::Source,
    utils::{Condvar, Mutex},
    SharedString,
};

#[cfg(doc)]
use crate::AssetCache;

pub use crate::key::{AssetKey, AssetType};
pub use watcher::FsWatcherBuilder;

pub(crate) use records::Dependencies;

pub(crate) type ReloadFn = fn(cache: crate::AnyCache, id: SharedString) -> Option<Dependencies>;

/// A message with an update of the state of the [`AssetCache`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateMessage {
    /// An asset was added to the cache.
    AddAsset(AssetKey),

    /// An asset was removed from the cache
    RemoveAsset(AssetKey),

    /// The cache was cleared.
    Clear,
}

enum CacheMessage {
    Ptr(NonNull<crate::cache::AssetMap>, NonNull<HotReloader>, usize),
    Static(&'static crate::cache::AssetMap, &'static HotReloader),

    Clear,
    AddCompound(CompoundReloadInfos),
}
unsafe impl Send for CacheMessage where crate::cache::AssetMap: Sync {}

/// An error returned when an end of a channel was disconnected.
#[derive(Debug)]
pub struct Disconnected;

enum Events {
    Single(AssetKey),
    Multiple(Vec<AssetKey>),
}

impl Events {
    fn for_each(self, mut f: impl FnMut(AssetKey)) {
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
    pub fn send(&self, event: AssetKey) -> Result<(), Disconnected> {
        self.0.send(Events::Single(event)).or(Err(Disconnected))
    }

    /// Sends multiple events an once.
    ///
    /// If successful, this function returns the number of events sent.
    pub fn send_multiple<I>(&self, events: I) -> Result<usize, Disconnected>
    where
        I: IntoIterator<Item = AssetKey>,
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

/// An `UpdateSender` that drops all incoming messages.
#[derive(Debug)]
pub struct UpdateSink;

impl UpdateSender for UpdateSink {
    fn send_update(&self, _message: UpdateMessage) {}
}

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

/// Used to make sure any thread calling `AssetCache::hot_reload` continues when
/// it is answered and not when another thread is. Using a channel would be
/// vulnerable to race condition, which is fine in that case but not really
/// future-proof.
#[derive(Default)]
struct Answers {
    next_token: AtomicUsize,
    current_token: Mutex<Option<usize>>,
    condvar: Condvar,
}

impl Answers {
    fn get_unique_token(&self) -> usize {
        self.next_token.fetch_add(1, Ordering::Relaxed)
    }

    fn notify(&self, token: usize) {
        let guard = self.current_token.lock();
        // Make sure everyone consumed its answer token
        let mut guard = self.condvar.wait_while(guard, |t| t.is_some());
        *guard = Some(token);
        self.condvar.notify_all();
    }

    fn wait_for_answer(&self, token: usize) {
        let guard = self.current_token.lock();
        let mut token = self.condvar.wait_while(guard, |t| *t != Some(token));
        *token = None;
    }
}

/// The hot-reloading handler.
pub(crate) struct HotReloader {
    sender: Sender<CacheMessage>,
    answers: Arc<Answers>,
    updates: DynUpdateSender,
}

impl HotReloader {
    /// Starts hot-reloading.
    fn start(
        events: Receiver<Events>,
        updates: DynUpdateSender,
        source: Box<dyn Source + Send>,
    ) -> Self {
        let (cache_msg_tx, cache_msg_rx) = channel::unbounded();
        let answers = Arc::new(Answers::default());
        let answers_clone = answers.clone();

        thread::Builder::new()
            .name("assets_hot_reload".to_string())
            .spawn(|| hot_reloading_thread(source, events, cache_msg_rx, answers_clone))
            .unwrap();

        Self {
            updates,
            sender: cache_msg_tx,
            answers,
        }
    }

    pub fn make<S: Source>(source: S) -> Option<Self> {
        let sent_source = source.make_source()?;
        let (events_tx, events_rx) = channel::unbounded();

        let updates = source
            .configure_hot_reloading(EventSender(events_tx))
            .map_err(|err| {
                log::error!("Unable to start hot-reloading: {err}");
            })
            .ok()?;

        Some(Self::start(events_rx, updates, sent_source))
    }

    // All theses methods ignore send/recv errors: the program can continue
    // without hot-reloading if it stopped, and an error should have already
    // been logged.

    pub(crate) fn add_asset(&self, id: SharedString, typ: AssetType) {
        let key = AssetKey { id, typ };
        self.updates.send_update(UpdateMessage::AddAsset(key));
    }

    pub(crate) fn remove_asset<A: Storable>(&self, id: SharedString) {
        if let Some(typ) = A::get_type::<crate::utils::Private>().to_asset_type() {
            let key = AssetKey { id, typ };
            self.updates.send_update(UpdateMessage::RemoveAsset(key));
        }
    }

    pub(crate) fn add_compound(
        &self,
        id: SharedString,
        deps: Dependencies,
        typ: Type,
        reload_fn: ReloadFn,
    ) {
        let infos = CompoundReloadInfos::from_type(id, deps, typ, reload_fn);
        let _ = self.sender.send(CacheMessage::AddCompound(infos));
    }

    pub(crate) fn clear(&self) {
        let _ = self.sender.send(CacheMessage::Clear);
        self.updates.send_update(UpdateMessage::Clear);
    }

    pub(crate) fn reload(&self, map: &crate::cache::AssetMap) {
        let token = self.answers.get_unique_token();
        if self
            .sender
            .send(CacheMessage::Ptr(
                NonNull::from(map),
                NonNull::from(self),
                token,
            ))
            .is_ok()
        {
            // When the hot-reloading thread is done, it sends back our back our token
            self.answers.wait_for_answer(token);
        }
    }

    pub(crate) fn send_static(&'static self, map: &'static crate::cache::AssetMap) {
        let _ = self.sender.send(CacheMessage::Static(map, self));
    }
}

impl fmt::Debug for HotReloader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("HotReloader { .. }")
    }
}

fn hot_reloading_thread(
    source: Box<dyn Source>,
    events: Receiver<Events>,
    cache_msg: Receiver<CacheMessage>,
    answers: Arc<Answers>,
) {
    log::info!("Starting hot-reloading");

    let mut cache = HotReloadingData::new(source);

    let mut select = channel::Select::new();
    select.recv(&cache_msg);
    select.recv(&events);

    loop {
        // We don't use `select` method here as we always want to check
        // `cache_msg` channel first.
        let ready = select.ready();

        loop {
            match cache_msg.try_recv() {
                Ok(CacheMessage::Ptr(ptr, reloader, token)) => {
                    // Safety: The received pointer is guaranteed to
                    // be valid until we reply back
                    unsafe {
                        cache.update_if_local(ptr.as_ref(), reloader.as_ref());
                    }
                    answers.notify(token);
                }
                Ok(CacheMessage::Static(asset_cache, reloader)) => {
                    cache.use_static_ref(asset_cache, reloader)
                }
                Ok(CacheMessage::Clear) => cache.clear_local_cache(),
                Ok(CacheMessage::AddCompound(infos)) => cache.add_compound(infos),
                Err(_) => break,
            }
        }

        if ready == 1 {
            match events.try_recv() {
                Ok(msg) => cache.load_asset(msg),
                Err(crossbeam_channel::TryRecvError::Empty) => (),
                // We won't receive events anymore, we can stop now
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }
    }

    log::info!("Stopping hot-reloading");
}
