pub mod dependencies;
mod paths;
mod watcher;

#[cfg(test)]
mod tests;

use paths::{CompoundReloadInfos, HotReloadingData};

use crossbeam_channel::{self as channel, Receiver, Sender};
use std::{fmt, path::PathBuf, ptr::NonNull, thread};

use crate::{
    utils::{HashSet, OwnedKey},
    AssetCache, SharedString,
};

use crate::key::AssetKey;

#[non_exhaustive]
enum UpdateMessage {
    AddAsset(AssetKey),
    Clear,
}

enum CacheMessage {
    Ptr(NonNull<AssetCache>),
    Static(&'static AssetCache),

    Clear,
    AddCompound(CompoundReloadInfos),
}
unsafe impl Send for CacheMessage where AssetCache: Sync {}

pub struct HotReloader {
    sender: Sender<CacheMessage>,
    receiver: Receiver<()>,
    updates: Sender<UpdateMessage>,
}

impl HotReloader {
    pub fn start(path: PathBuf) -> Result<Self, notify::Error> {
        let (msg_tx, msg_rx) = channel::unbounded();
        let (answer_tx, answer_rx) = channel::unbounded();
        let (updates_tx, updates_rx) = channel::unbounded();

        let (watcher, events_rx) = watcher::make(path.clone(), updates_rx)?;

        thread::Builder::new()
            .name("assets_hot_reload".to_string())
            .spawn(|| Self::hot_reloading_thread(path, watcher, events_rx, msg_rx, answer_tx))
            .unwrap();

        Ok(HotReloader {
            updates: updates_tx,
            sender: msg_tx,
            receiver: answer_rx,
        })
    }

    // this is done in a new thread
    fn hot_reloading_thread(
        root: PathBuf,
        _watcher: notify::RecommendedWatcher,
        events_rx: Receiver<AssetKey>,
        ptr_rx: Receiver<CacheMessage>,
        answer_tx: Sender<()>,
    ) {
        log::trace!("Starting hot-reloading");

        let mut cache = HotReloadingData::new(root);

        // At the beginning, we select over three channels:
        // - One to notify that we can update the `AssetCache` or that we
        //   can switch to using a 'static reference. We close this channel
        //   in the latter case.
        // - One to receive events from notify
        // - One to update the watched paths list when the `AssetCache`
        //   changes
        let mut select = channel::Select::new();
        select.recv(&ptr_rx);
        select.recv(&events_rx);

        loop {
            let ready = select.select();
            match ready.index() {
                0 => match ready.recv(&ptr_rx) {
                    Ok(CacheMessage::Ptr(ptr)) => {
                        // Safety: The received pointer is guaranteed to
                        // be valid until we reply back
                        cache.update_if_local(unsafe { ptr.as_ref() });
                        answer_tx.send(()).unwrap();
                    }
                    Ok(CacheMessage::Static(asset_cache)) => cache.use_static_ref(asset_cache),
                    Ok(CacheMessage::Clear) => cache.clear_local_cache(),
                    Ok(CacheMessage::AddCompound(infos)) => cache.add_compound(infos),
                    Err(_) => (),
                },

                1 => match ready.recv(&events_rx) {
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

    pub(crate) fn reload(&self, cache: &AssetCache) {
        let _ = self.sender.send(CacheMessage::Ptr(cache.into()));
        let _ = self.receiver.recv();
    }

    pub(crate) fn send_static(&self, cache: &'static AssetCache) {
        let _ = self.sender.send(CacheMessage::Static(cache));
    }
}

impl fmt::Debug for HotReloader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("HotReloader { .. }")
    }
}
