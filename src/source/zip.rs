use super::{DirEntry, Source};
use crate::{
    utils::{extension_of, HashMap},
    SharedBytes,
};

use std::{
    fmt, hash, io,
    path::{self, Path},
    sync::Arc,
};

use sync_file::SyncFile;
use zip::{read::ZipFile, ZipArchive};

#[derive(Clone, Hash, PartialEq, Eq)]
struct FileDesc(Arc<(String, String)>);

impl fmt::Debug for FileDesc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("FileDesc")
            .field("id", &self.0 .0)
            .field("ext", &self.0 .1)
            .finish()
    }
}

/// This hack enables us to use a `(&str, &str)` as a key for an HashMap without
/// allocating a `FileDesc`
trait FileKey {
    fn id(&self) -> &str;
    fn ext(&self) -> &str;
}

impl FileKey for FileDesc {
    fn id(&self) -> &str {
        &self.0 .0
    }

    fn ext(&self) -> &str {
        &self.0 .1
    }
}

impl FileKey for (&'_ str, &'_ str) {
    fn id(&self) -> &str {
        self.0
    }

    fn ext(&self) -> &str {
        self.1
    }
}

impl<'a> std::borrow::Borrow<dyn FileKey + 'a> for FileDesc {
    fn borrow(&self) -> &(dyn FileKey + 'a) {
        self
    }
}

impl PartialEq for dyn FileKey + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id() && self.ext() == other.ext()
    }
}

impl Eq for dyn FileKey + '_ {}

impl hash::Hash for dyn FileKey + '_ {
    fn hash<H: hash::Hasher>(&self, hasher: &mut H) {
        self.id().hash(hasher);
        self.ext().hash(hasher);
    }
}

/// An entry in a archive directory.
#[derive(Debug)]
enum OwnedEntry {
    File(FileDesc),
    Dir(String),
}

impl OwnedEntry {
    fn as_dir_entry(&self) -> DirEntry {
        match self {
            OwnedEntry::File(FileDesc(desc)) => DirEntry::File(&desc.0, &desc.1),
            OwnedEntry::Dir(id) => DirEntry::Directory(id),
        }
    }
}

/// Build ids from components.
///
/// Using this allows to easily reuse buffers when building several ids in a
/// row, and thus to avoid repeated allocations.
#[derive(Default)]
struct IdBuilder {
    segments: Vec<String>,
    len: usize,
}

impl IdBuilder {
    /// Pushs a segment in the builder.
    #[inline]
    fn push(&mut self, s: &str) {
        match self.segments.get_mut(self.len) {
            Some(seg) => {
                seg.clear();
                seg.push_str(s);
            }
            None => self.segments.push(s.to_owned()),
        }
        self.len += 1;
    }

    /// Pops a segment from the builder.
    ///
    /// Returns `None` if the builder was empty.
    #[inline]
    fn pop(&mut self) -> Option<()> {
        self.len = self.len.checked_sub(1)?;
        Some(())
    }

    /// Joins segments to build a id.
    #[inline]
    fn join(&self) -> String {
        self.segments[..self.len].join(".")
    }

    /// Resets the builder without freeing buffers.
    #[inline]
    fn reset(&mut self) {
        self.len = 0;
    }
}

/// Register a file of an archive in maps.
fn register_file(
    file: ZipFile,
    index: usize,
    files: &mut HashMap<FileDesc, usize>,
    dirs: &mut HashMap<String, Vec<OwnedEntry>>,
    id_builder: &mut IdBuilder,
) {
    id_builder.reset();

    // Check the path.
    let path = match file.enclosed_name() {
        Some(path) => path,
        None => {
            log::warn!("Suspicious path in zip archive: {:?}", file.name());
            return;
        }
    };

    // Parse the path and register it.
    // The closure is used as a cheap `try` block.
    let ok = (|| {
        // Fill `id_builder` from the parent's components
        let parent = path.parent()?;
        for comp in parent.components() {
            match comp {
                path::Component::Normal(s) => {
                    let segment = s.to_str()?;
                    if segment.contains('.') {
                        return None;
                    }
                    id_builder.push(segment);
                }
                path::Component::ParentDir => id_builder.pop()?,
                path::Component::CurDir => continue,
                _ => return None,
            }
        }

        // Build the ids of the file and its parent.
        let parent_id = id_builder.join();
        id_builder.push(path.file_stem()?.to_str()?);
        let id = id_builder.join();

        // Register the file in the maps.
        let entry = if file.is_file() {
            let ext = extension_of(path)?.to_owned();
            let desc = FileDesc(Arc::new((id, ext)));
            files.insert(desc.clone(), index);
            OwnedEntry::File(desc)
        } else {
            if !dirs.contains_key(&id) {
                dirs.insert(id.clone(), Vec::new());
            }
            OwnedEntry::Dir(id)
        };
        dirs.entry(parent_id).or_insert_with(Vec::new).push(entry);

        Some(())
    })()
    .is_some();

    if !ok {
        log::warn!("Unsupported path in zip archive: {path:?}");
    }
}

/// A [`Source`] to load assets from a zip archive.
///
/// The archive can be backed by any reader that also implements [`io::Seek`]
/// and [`Clone`].
///
/// **Warning**: This will clone the reader each time it is read, so you should
/// ensure that is cheap to clone (eg *not* `Vec<u8>`).
#[cfg_attr(docsrs, doc(cfg(feature = "zip")))]
pub struct Zip<R = SyncFile> {
    files: HashMap<FileDesc, usize>,
    dirs: HashMap<String, Vec<OwnedEntry>>,
    archive: ZipArchive<R>,
}

impl Zip<SyncFile> {
    /// Creates a `Zip` archive backed by the file at the given path.
    #[inline]
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = SyncFile::open(path)?;
        Zip::from_reader(file)
    }
}

impl Zip<io::Cursor<SharedBytes>> {
    /// Creates a `Zip` archive backed by a byte buffer in memory.
    ///
    /// If you want to use another kind of byte buffer (such as `&[u8]`), you
    /// can use `from_reader`.
    #[inline]
    pub fn from_bytes(bytes: SharedBytes) -> io::Result<Self> {
        Zip::from_reader(io::Cursor::new(bytes))
    }
}

impl<'a> Zip<io::Cursor<&'a [u8]>> {
    /// Creates a `Zip` archive backed by a byte buffer in memory.
    ///
    /// If you want to use another kind of byte buffer (such as `Arc<[u8]>`),
    /// you can use `from_reader`.
    #[inline]
    pub fn from_slice(bytes: &'a [u8]) -> io::Result<Self> {
        Zip::from_reader(io::Cursor::new(bytes))
    }
}

impl<R> Zip<R>
where
    R: io::Read + io::Seek,
{
    /// Creates a `Zip` archive backed by a reader that supports seeking.
    pub fn from_reader(reader: R) -> io::Result<Zip<R>> {
        let mut archive = ZipArchive::new(reader)?;

        let len = archive.len();
        let mut files = HashMap::with_capacity(len);
        let mut dirs = HashMap::new();
        let mut id_builder = IdBuilder::default();

        for index in 0..len {
            let file = archive.by_index(index)?;
            register_file(file, index, &mut files, &mut dirs, &mut id_builder);
        }

        Ok(Zip {
            files,
            dirs,
            archive,
        })
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "zip")))]
impl<R> Source for Zip<R>
where
    R: io::Read + io::Seek + Clone,
{
    fn read(&self, id: &str, ext: &str) -> io::Result<super::FileContent> {
        use io::Read;

        // Get the file within the archive
        let key: &dyn FileKey = &(id, ext);
        let id = *self.files.get(key).ok_or(io::ErrorKind::NotFound)?;
        let mut archive = self.archive.clone();
        let mut file = archive.by_index(id)?;

        // Read it in a buffer
        let mut content = Vec::with_capacity(file.size() as usize + 1);
        file.read_to_end(&mut content)?;

        Ok(super::FileContent::Buffer(content))
    }

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        let dir = self.dirs.get(id).ok_or(io::ErrorKind::NotFound)?;
        dir.iter().map(OwnedEntry::as_dir_entry).for_each(f);
        Ok(())
    }

    fn exists(&self, entry: DirEntry) -> bool {
        match entry {
            DirEntry::File(id, ext) => self.files.contains_key(&(id, ext) as &dyn FileKey),
            DirEntry::Directory(id) => self.dirs.contains_key(id),
        }
    }
}

impl<R> fmt::Debug for Zip<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Zip").field("dirs", &self.dirs).finish()
    }
}
