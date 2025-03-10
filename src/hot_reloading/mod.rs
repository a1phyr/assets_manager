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

use paths::{AssetReloadInfos, HotReloadingData};

use crossbeam_channel::{self as channel, Receiver, Sender};
use std::{
    fmt,
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use crate::{
    SharedString,
    key::Type,
    source::{OwnedDirEntry, Source},
    utils::{Condvar, Mutex},
};

#[cfg(doc)]
use crate::AssetCache;

pub use watcher::FsWatcherBuilder;

pub(crate) use records::{BorrowedDependency, Dependencies, Dependency};

enum CacheMessage {
    Ptr(NonNull<crate::cache::AssetMap>, NonNull<HotReloader>, usize),
    Static(&'static crate::cache::AssetMap, &'static HotReloader),

    Clear,
    AddAsset(AssetReloadInfos),
}
unsafe impl Send for CacheMessage where crate::cache::AssetMap: Sync {}

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
}

impl HotReloader {
    /// Starts hot-reloading.
    fn start(events: Receiver<Events>, source: Box<dyn Source + Send>) -> Self {
        let (cache_msg_tx, cache_msg_rx) = channel::unbounded();
        let answers = Arc::new(Answers::default());
        let answers_clone = answers.clone();

        thread::Builder::new()
            .name("assets_hot_reload".to_string())
            .spawn(|| hot_reloading_thread(source, events, cache_msg_rx, answers_clone))
            .unwrap();

        Self {
            sender: cache_msg_tx,
            answers,
        }
    }

    pub fn make<S: Source>(source: S) -> Option<Self> {
        let sent_source = source.make_source()?;
        let (events_tx, events_rx) = channel::unbounded();

        source
            .configure_hot_reloading(EventSender(events_tx))
            .map_err(|err| {
                log::error!("Unable to start hot-reloading: {err}");
            })
            .ok()?;

        Some(Self::start(events_rx, sent_source))
    }

    // All theses methods ignore send/recv errors: the program can continue
    // without hot-reloading if it stopped, and an error should have already
    // been logged.

    pub(crate) fn add_asset(&self, id: SharedString, deps: Dependencies, typ: Type) {
        let infos = AssetReloadInfos::from_type(id, deps, typ);
        let _ = self.sender.send(CacheMessage::AddAsset(infos));
    }

    pub(crate) fn clear(&self) {
        let _ = self.sender.send(CacheMessage::Clear);
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
                Ok(CacheMessage::AddAsset(infos)) => cache.add_asset(infos),
                Err(_) => break,
            }
        }

        if ready == 1 {
            match events.try_recv() {
                Ok(msg) => cache.handle_events(msg),
                Err(crossbeam_channel::TryRecvError::Empty) => (),
                // We won't receive events anymore, we can stop now
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }
    }

    log::info!("Stopping hot-reloading");
}
