use super::{DirEntry, Source};
use crate::{
    utils::{extension_of, HashMap, IdBuilder},
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
        dirs.entry(parent_id).or_default().push(entry);

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
    label: Option<String>,
}

impl Zip<SyncFile> {
    /// Creates a `Zip` archive backed by the file at the given path.
    #[inline]
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::_open(path.as_ref())
    }

    #[inline]
    fn _open(path: &Path) -> io::Result<Self> {
        let file = SyncFile::open(path)?;
        Zip::from_reader_with_label(file, path.display().to_string())
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

    /// Creates a `Zip` archive backed by a byte buffer in memory.
    ///
    /// An additionnal label that will be used in errors can be added.
    #[inline]
    pub fn from_bytes_with_label(bytes: SharedBytes, label: String) -> io::Result<Self> {
        Zip::from_reader_with_label(io::Cursor::new(bytes), label)
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

    /// Creates a `Zip` archive backed by a byte buffer in memory.
    ///
    /// An additionnal label that will be used in errors can be added.
    #[inline]
    pub fn from_slice_with_label(bytes: &'a [u8], label: String) -> io::Result<Self> {
        Zip::from_reader_with_label(io::Cursor::new(bytes), label)
    }
}

impl<R> Zip<R>
where
    R: io::Read + io::Seek,
{
    /// Creates a `Zip` archive backed by a reader that supports seeking.
    pub fn from_reader(reader: R) -> io::Result<Zip<R>> {
        Self::create(reader, None)
    }

    /// Creates a `Zip` archive backed by a reader that supports seeking.
    ///
    /// An additionnal label that will be used in errors can be added.
    pub fn from_reader_with_label(reader: R, label: String) -> io::Result<Zip<R>> {
        Self::create(reader, Some(label))
    }

    fn create(reader: R, label: Option<String>) -> io::Result<Zip<R>> {
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
            label,
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
        let index = *self
            .files
            .get(key)
            .ok_or_else(|| error::find_file(id, &self.label))?;
        let mut archive = self.archive.clone();
        let mut file = archive
            .by_index(index)
            .map_err(|err| error::open_file(err, id, &self.label))?;

        // Read it in a buffer
        let mut content = Vec::with_capacity(file.size() as usize + 1);
        file.read_to_end(&mut content)
            .map_err(|err| error::read_file(err, id, &self.label))?;

        Ok(super::FileContent::Buffer(content))
    }

    fn read_dir(&self, id: &str, f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
        let dir = self
            .dirs
            .get(id)
            .ok_or_else(|| error::find_dir(id, &self.label))?;
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

mod error {
    use std::{fmt, io};
    use zip::result::ZipError;

    #[cold]
    pub fn find_file(id: &str, label: &Option<String>) -> io::Error {
        let msg = match label {
            Some(lbl) => format!("Could not find asset \"{id}\" in {lbl}"),
            None => format!("Could not find asset \"{id}\" in ZIP"),
        };

        io::Error::new(io::ErrorKind::NotFound, msg)
    }

    #[cold]
    pub fn read_file(err: io::Error, id: &str, label: &Option<String>) -> io::Error {
        #[derive(Debug)]
        struct Error {
            err: io::Error,
            msg: String,
        }
        impl fmt::Display for Error {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.msg)
            }
        }
        impl std::error::Error for Error {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.err)
            }
        }

        let msg = match label {
            Some(lbl) => format!("Could not read \"{id}\" in {lbl}"),
            None => format!("Could not read \"{id}\" in ZIP"),
        };

        io::Error::new(err.kind(), Error { err, msg })
    }

    #[cold]
    pub fn open_file(err: ZipError, id: &str, label: &Option<String>) -> io::Error {
        #[derive(Debug)]
        struct Error {
            err: ZipError,
            msg: String,
        }
        impl fmt::Display for Error {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.msg)
            }
        }
        impl std::error::Error for Error {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.err)
            }
        }

        let msg = match label {
            Some(lbl) => format!("Could not open \"{id}\" in {lbl}"),
            None => format!("Could not open \"{id}\" in ZIP"),
        };

        let kind = match &err {
            ZipError::Io(err) => err.kind(),
            ZipError::InvalidArchive(_) => io::ErrorKind::InvalidData,
            ZipError::UnsupportedArchive(_) => io::ErrorKind::Unsupported,
            ZipError::FileNotFound => io::ErrorKind::NotFound,
        };

        io::Error::new(kind, Error { err, msg })
    }

    #[cold]
    pub fn find_dir(id: &str, label: &Option<String>) -> io::Error {
        let msg = match label {
            Some(lbl) => format!("Could not find directory \"{id}\" in {lbl}"),
            None => format!("Could not find directory \"{id}\" in ZIP"),
        };

        io::Error::new(io::ErrorKind::NotFound, msg)
    }
}
