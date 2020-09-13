mod paths;

#[cfg(test)]
mod tests;

pub use paths::UpdateMessage;
use paths::HotReloadingData;

use crossbeam_channel::{self as channel, Receiver, Sender};

use std::{
    fmt,
    path::Path,
    ptr::NonNull,
    sync::mpsc,
    thread,
    time::Duration,
};

use notify::{DebouncedEvent, RecursiveMode, Watcher};

use crate::{AssetCache, utils::Mutex};


enum Message {
    Ptr(NonNull<AssetCache>),
    Static(&'static AssetCache),
}
unsafe impl Send for Message where AssetCache: Sync {}


fn std_crossbeam_channel<T: Send + 'static>() -> (mpsc::Sender<T>, Receiver<T>) {
    let (std_tx, std_rx) = mpsc::channel();
    let (crossbeam_tx, crossbeam_rx) = channel::unbounded();

    thread::spawn(move || {
        while let Ok(msg) = std_rx.recv() {
            if crossbeam_tx.send(msg).is_err() {
                break;
            }
        }
    });

    (std_tx, crossbeam_rx)
}


struct Client {
    sender: Sender<Message>,
    receiver: Receiver<()>,
}

pub struct HotReloader {
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

        thread::spawn(move || {
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
                        Ok(Message::Ptr(ptr)) => {
                            if let Some(cache) = cache.local_cache() {
                                // Safety: The received pointer is guarantied to
                                // be valid until we reply back
                                cache.update(unsafe { ptr.as_ref() });
                                answer_tx.send(()).unwrap();
                            }
                        },
                        Ok(Message::Static(asset_cache)) => {
                            cache.use_static_ref(asset_cache);
                            select.remove(0);
                        },
                        Err(_) => (),
                    },

                    1 => match ready.recv(&notify_rx) {
                        Ok(event) => match event {
                            DebouncedEvent::Write(path)
                            | DebouncedEvent::Chmod(path)
                            | DebouncedEvent::Create(path) => {
                                cache.load(path);
                            },
                            DebouncedEvent::Remove(path) => {
                                cache.remove(path);
                            },
                            DebouncedEvent::Rename(src, dst) => {
                                cache.load(dst);
                                cache.remove(src);
                            },
                            _ => (),
                        },
                        Err(_) => {
                            log::error!("Notify panicked, hot-reloading stopped");
                            break;
                        },
                    },

                    2 => match ready.recv(&updates_rx) {
                        Ok(msg) => cache.paths.update(msg),
                        Err(_) => break,
                    },

                    _ => unreachable!(),
                }
            }
        });

        Ok(HotReloader {
            updates: updates_tx,

            channel: Mutex::new(Some(Client {
                sender: ptr_tx,
                receiver: answer_rx,
            })),
        })
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
            let _ = sender.send(Message::Ptr(cache.into()));
            let _ = receiver.recv();
        }
    }

    pub fn send_static(&self, cache: &'static AssetCache) {
        let mut lock = self.channel.lock();

        if let Some(Client { sender, .. }) = &mut *lock {
            let _ = sender.send(Message::Static(cache));
            *lock = None;
        }
    }
}

impl fmt::Debug for HotReloader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("HotReloader { .. }")
    }
}
