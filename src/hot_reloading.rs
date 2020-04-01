use std::{
    collections::HashMap,
    io,
    fmt,
    fs,
    mem::ManuallyDrop,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::mpsc::{self, channel, Receiver, Sender},
    thread,
    time::Duration,
};

use notify::{DebouncedEvent, RecursiveMode, Watcher};

use crate::AssetCache;


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


struct FileCache {
    cache: HashMap<PathBuf, io::Result<Vec<u8>>>,
}

impl FileCache {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    fn load(&mut self, path: PathBuf) {
        let content = fs::read(&path);

        match self.cache.get_mut(&path) {
            Some(Ok(ref mut cached)) => if let Ok(content) = content {
                *cached = content;
            },
            Some(res) => *res = content,
            None => {
                self.cache.insert(path, content);
            }
        }
    }

    fn update(&mut self, cache: &AssetCache) {
        for (path, content) in self.cache.drain() {
            match content {
                Ok(content) => { reload(cache, path, content); },
                Err(err) => log::warn!("Cannot reload {:?}: {}", path, err),
            }
        }
    }
}

fn path_to_id(path: &Path) -> Option<String> {
    let mut id = String::with_capacity(path.as_os_str().len());
    let mut iter = path.iter();
    let mut next = iter.next()?;

    loop {
        let cur = next;

        match iter.next() {
            Some(item) => {
                id.push_str(cur.to_str()?);
                id.push('.');
                next = item;
            },
            None => {
                let file = Path::new(cur).file_stem()?;
                id.push_str(file.to_str()?);
                return Some(id);
            }
        }
    }
}

fn reload(cache: &AssetCache, path: PathBuf, content: Vec<u8>) -> Option<()> {
    let p = path.strip_prefix(cache.path()).ok()?;

    let extension = match p.extension() {
        Some(e) => e.to_str()?,
        None => "",
    };

    cache.reload(&path_to_id(p)?, extension, content);

    Some(())
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
    pub fn start(cache: &AssetCache) -> Result<Self, notify::Error> {
        let (notify_tx, notify_rx) = channel();

        let (ptr_tx, ptr_rx) = channel();
        let (answer_tx, answer_rx) = channel();

        let mut watcher = notify::watcher(notify_tx, Duration::from_millis(50))?;
        watcher.watch(cache.path(), RecursiveMode::Recursive)?;

        let handle = thread::spawn(move || {
            const TIMEOUT: Duration = Duration::from_millis(10);
            let mut cache = FileCache::new();

            loop {
                match ptr_rx.recv_timeout(TIMEOUT) {
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                    Ok(SharedPtr(ptr)) => {
                        // Safety: The received pointer is guarantied to be valid
                        // until we reply back
                        cache.update(unsafe { ptr.as_ref() });
                        answer_tx.send(()).unwrap();
                    },
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }

                while let Ok(event) = notify_rx.try_recv() {
                    match event {
                        DebouncedEvent::Write(path)
                        | DebouncedEvent::Chmod(path)
                        | DebouncedEvent::Create(path)
                        | DebouncedEvent::Rename(_, path) => {
                            cache.load(path);
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
