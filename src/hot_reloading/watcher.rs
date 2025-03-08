use crate::{BoxedError, source::OwnedDirEntry, utils::IdBuilder};
use std::{
    fmt,
    path::{self, Path, PathBuf},
};

#[cfg(doc)]
use crate::source::Source;

/// Built-in reloader based on filesystem events.
///
/// You can use it to quickly set up hot-reloading for a custom [`Source`].
pub struct FsWatcherBuilder {
    roots: Vec<PathBuf>,
    watcher: notify::RecommendedWatcher,
    payload_sender: crossbeam_channel::Sender<NotifyEventHandler>,
}

impl FsWatcherBuilder {
    /// Creates a new builder.
    pub fn new() -> Result<Self, BoxedError> {
        let (payload_sender, payload_receiver) = crossbeam_channel::unbounded();
        let watcher = notify::recommended_watcher(EventHandlerPayload::new(payload_receiver))?;

        Ok(Self {
            roots: Vec::new(),
            watcher,
            payload_sender,
        })
    }

    /// Adds a path to watch.
    pub fn watch(&mut self, path: PathBuf) -> Result<(), BoxedError> {
        notify::Watcher::watch(&mut self.watcher, &path, notify::RecursiveMode::Recursive)?;
        self.roots.push(path);
        Ok(())
    }

    /// Starts the watcher.
    pub fn build(self, events: super::EventSender) {
        let event_handler = NotifyEventHandler {
            roots: self.roots,
            events,
            id_builder: IdBuilder::default(),

            watcher: Some(self.watcher),
        };

        let _ = self.payload_sender.send(event_handler);
    }
}

impl fmt::Debug for FsWatcherBuilder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("FsWatcherBuilder")
            .field("roots", &self.roots)
            .finish()
    }
}

fn id_of_path(id_builder: &mut IdBuilder, root: &Path, path: &Path) -> Option<OwnedDirEntry> {
    id_builder.reset();

    for comp in path.parent()?.strip_prefix(root).ok()?.components() {
        match comp {
            path::Component::Normal(s) => id_builder.push(s.to_str()?)?,
            path::Component::ParentDir => id_builder.pop()?,
            path::Component::CurDir => continue,
            _ => return None,
        }
    }

    // Build the id of the file.
    id_builder.push(path.file_stem()?.to_str()?)?;
    let id = id_builder.join();

    let entry = if path.is_dir() {
        OwnedDirEntry::Directory(id)
    } else {
        let ext = crate::utils::extension_of(path)?.into();
        OwnedDirEntry::File(id, ext)
    };

    Some(entry)
}

enum EventHandlerPayload<H> {
    Waiting(crossbeam_channel::Receiver<H>),
    Handler(H),
}

impl<H: notify::EventHandler> EventHandlerPayload<H> {
    fn new(receiver: crossbeam_channel::Receiver<H>) -> Self {
        Self::Waiting(receiver)
    }
}

impl<H: notify::EventHandler> notify::EventHandler for EventHandlerPayload<H> {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        match self {
            Self::Waiting(recv) => {
                if let Ok(mut handler) = recv.try_recv() {
                    handler.handle_event(event);
                    *self = Self::Handler(handler);
                }
            }
            Self::Handler(handler) => handler.handle_event(event),
        }
    }
}

struct NotifyEventHandler {
    roots: Vec<PathBuf>,
    events: super::EventSender,
    id_builder: IdBuilder,

    watcher: Option<notify::RecommendedWatcher>,
}

impl notify::EventHandler for NotifyEventHandler {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        match event {
            Ok(event) => {
                log::trace!("Received filesystem event: {event:?}");

                for path in event.paths {
                    let paths = match event.kind {
                        notify::EventKind::Any | notify::EventKind::Modify(_) => vec![&*path],
                        notify::EventKind::Create(_) => match path.parent() {
                            Some(parent) => vec![&path, parent],
                            None => vec![&*path],
                        },
                        notify::EventKind::Remove(_) => match path.parent() {
                            Some(parent) => vec![parent],
                            None => vec![],
                        },
                        notify::EventKind::Access(_) | notify::EventKind::Other => return,
                    };
                    let ids = paths
                        .into_iter()
                        .flat_map(|p| self.roots.iter().map(move |r| (p, r)))
                        .filter_map(|(path, root)| id_of_path(&mut self.id_builder, root, path));

                    if self.events.send_multiple(ids).is_err() {
                        drop(self.watcher.take());
                    }
                }
            }
            Err(err) => log::warn!("Error from notify: {err}"),
        }
    }
}
