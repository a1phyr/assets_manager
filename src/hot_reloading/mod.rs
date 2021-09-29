mod paths;

pub mod dependencies;

#[cfg(test)]
mod tests;

use paths::HotReloadingData;
pub(crate) use paths::{AssetReloadInfos, CompoundReloadInfos, UpdateMessage};

use crossbeam_channel::{self as channel, Receiver, Sender};

use std::{fmt, path::Path, ptr::NonNull, sync::mpsc, thread, time::Duration};

use notify::{DebouncedEvent, RecursiveMode, Watcher};

use crate::{utils::Mutex, AssetCache};

enum CacheMessage {
    Ptr(NonNull<AssetCache>),
    Static(&'static AssetCache),
}
unsafe impl Send for CacheMessage where AssetCache: Sync {}

fn std_crossbeam_channel<T: Send + 'static>() -> (mpsc::Sender<T>, Receiver<T>) {
    let (std_tx, std_rx) = mpsc::channel();
    let (crossbeam_tx, crossbeam_rx) = channel::unbounded();

    thread::Builder::new()
        .name("assets_workaround".to_string())
        .spawn(|| workaround_channels(std_rx, crossbeam_tx))
        .unwrap();

    (std_tx, crossbeam_rx)
}

fn workaround_channels<T: Send + 'static>(std_rx: mpsc::Receiver<T>, crossbeam_tx: Sender<T>) {
    while let Ok(msg) = std_rx.recv() {
        if crossbeam_tx.send(msg).is_err() {
            break;
        }
    }
}

struct Client {
    sender: Sender<CacheMessage>,
    receiver: Receiver<()>,
}

pub(crate) struct HotReloader {
    channel: Mutex<Option<Client>>,
    updates: Sender<UpdateMessage>,
}

impl HotReloader {
    pub fn start(path: &Path) -> Result<Self, notify::Error> {
        let (notify_tx, notify_rx) = std_crossbeam_channel();

        let (ptr_tx, ptr_rx) = channel::unbounded();
        let (answer_tx, answer_rx) = channel::unbounded();
        let (updates_tx, updates_rx) = channel::unbounded();

        let mut watcher = notify::watcher(notify_tx, Duration::from_millis(50))?;
        watcher.watch(path, RecursiveMode::Recursive)?;

        thread::Builder::new()
            .name("assets_hot_reload".to_string())
            .spawn(|| Self::hot_reloading_thread(watcher, notify_rx, ptr_rx, answer_tx, updates_rx))
            .unwrap();

        Ok(HotReloader {
            updates: updates_tx,

            channel: Mutex::new(Some(Client {
                sender: ptr_tx,
                receiver: answer_rx,
            })),
        })
    }

    // this is done in a new thread
    fn hot_reloading_thread(
        watcher: notify::RecommendedWatcher,
        notify_rx: Receiver<DebouncedEvent>,
        ptr_rx: Receiver<CacheMessage>,
        answer_tx: Sender<()>,
        updates_rx: Receiver<UpdateMessage>,
    ) {
        log::trace!("Starting hot-reloading");

        // Keep the notify Watcher alive as long as the thread is running
        let _watcher = watcher;

        // At the beginning, we select over three channels:
        // - One to notify that we can update the `AssetCache` or that we
        //   can switch to using a 'static reference. We close this channel
        //   in the latter case.
        // - One to receive events from notify
        // - One to update the watched paths list when the `AssetCache`
        //   changes
        let mut select = channel::Select::new();
        select.recv(&ptr_rx);
        select.recv(&notify_rx);
        select.recv(&updates_rx);

        let mut cache = HotReloadingData::new();

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
                    Ok(CacheMessage::Static(asset_cache)) => {
                        cache.use_static_ref(asset_cache);
                        select.remove(0);
                    }
                    Err(_) => (),
                },

                1 => match ready.recv(&notify_rx) {
                    Ok(event) => match event {
                        DebouncedEvent::Write(path)
                        | DebouncedEvent::Chmod(path)
                        | DebouncedEvent::Rename(_, path)
                        | DebouncedEvent::Create(path) => {
                            cache.load(path);
                        }
                        _ => (),
                    },
                    Err(_) => {
                        log::error!("Notify panicked, hot-reloading stopped");
                        break;
                    }
                },

                2 => match ready.recv(&updates_rx) {
                    Ok(msg) => cache.recv_update(msg),
                    Err(_) => break,
                },

                _ => unreachable!(),
            }
        }
    }

    // All theses methods ignore send/recv errors: the program can continue
    // without hot-reloading if it stopped, and an error should have already
    // been logged.

    pub fn send_update(&self, msg: UpdateMessage) {
        let _ = self.updates.send(msg);
    }

    pub fn reload(&self, cache: &AssetCache) {
        let lock = self.channel.lock();

        if let Some(Client { sender, receiver }) = &*lock {
            let _ = sender.send(CacheMessage::Ptr(cache.into()));
            let _ = receiver.recv();
        }
    }

    pub fn send_static(&self, cache: &'static AssetCache) {
        let mut lock = self.channel.lock();

        if let Some(Client { sender, .. }) = &mut *lock {
            let _ = sender.send(CacheMessage::Static(cache));
            *lock = None;
        }
    }
}

impl fmt::Debug for HotReloader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("HotReloader { .. }")
    }
}
