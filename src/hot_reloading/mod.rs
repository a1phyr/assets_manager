mod paths;

pub(crate) use paths::WatchedPaths;
use paths::FileCache;

use std::{
    fmt,
    mem::ManuallyDrop,
    ptr::NonNull,
    sync::mpsc::{self, channel, Receiver, Sender},
    thread,
    time::Duration,
};

use notify::{DebouncedEvent, RecursiveMode, Watcher};

use crate::{
    AssetCache,
};


struct SharedPtr<T>(NonNull<T>);
unsafe impl<T: Sync> Send for SharedPtr<T> {}


struct JoinOnDrop(ManuallyDrop<thread::JoinHandle<()>>);

impl Drop for JoinOnDrop {
    fn drop(&mut self) {
        unsafe {
            let _ = ManuallyDrop::take(&mut self.0).join();
        }
    }
}

impl From<thread::JoinHandle<()>> for JoinOnDrop {
    fn from(handle: thread::JoinHandle<()>) -> Self {
        Self(ManuallyDrop::new(handle))
    }
}


#[allow(unused)]
pub struct HotReloader {
    sender: Sender<SharedPtr<AssetCache>>,
    receiver: Receiver<()>,

    // The Watcher has to be dropped before the JoinHandle, so the spawned
    // thread can be notified that it should end before we join on it
    watcher: notify::RecommendedWatcher,
    handle: JoinOnDrop,
}


impl HotReloader {
    pub fn start(cache: &AssetCache) -> Result<Self, HotReloadingError> {
        let (notify_tx, notify_rx) = channel();

        let (ptr_tx, ptr_rx) = channel();
        let (answer_tx, answer_rx) = channel();

        let mut watcher = notify::watcher(notify_tx, Duration::from_millis(50))?;
        watcher.watch(cache.path(), RecursiveMode::Recursive)?;

        let handle = thread::spawn(move || {
            const TIMEOUT: Duration = Duration::from_millis(20);
            let mut cache = FileCache::new();

            loop {
                match ptr_rx.recv_timeout(TIMEOUT) {
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                    Ok(SharedPtr(ptr)) => {
                        {
                            // Safety: The received pointer is guarantied to be
                            // valid until we reply back
                            let asset_cache = unsafe { ptr.as_ref() };
                            cache.update(asset_cache);
                            cache.get_watched(&mut asset_cache.watched.lock());
                        }
                        answer_tx.send(()).unwrap();
                    },
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }

                while let Ok(event) = notify_rx.try_recv() {
                    match event {
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
                    }
                }
            }
        }).into();

        Ok(HotReloader {
            watcher,
            handle,

            sender: ptr_tx,
            receiver: answer_rx,
        })
    }

    pub fn reload(&self, cache: &AssetCache) {
        self.sender.send(SharedPtr(cache.into())).unwrap();
        self.receiver.recv().unwrap();
    }
}

impl fmt::Debug for HotReloader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("HotReloader { .. }")
    }
}


/// An error which occurs when starting hot-reoading.
///
/// This error can be returned by [`AssetCache::hot_reload`]
///
/// [`AssetCache::hot_reload`]: struct.AssetCache.html#method.hot_reload
#[cfg_attr(docsrs, doc(cfg(feature = "hot-reloading")))]
#[derive(Debug)]
pub struct HotReloadingError(notify::Error);

#[doc(hidden)]
impl From<notify::Error> for HotReloadingError {
    fn from(err: notify::Error) -> Self { Self(err) }
}

impl fmt::Display for HotReloadingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for HotReloadingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            notify::Error::Io(err) => Some(err),
            _ => None,
        }
    }
}
