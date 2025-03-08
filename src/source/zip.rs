#[cfg(feature = "mmap")]
use super::ArcMap;
use super::{DirEntry, Source};
use crate::{
    SharedString,
    utils::{HashMap, IdBuilder, extension_of},
};
use std::{fmt, io, path};
use sync_file::SyncFile;
use zip::{ZipArchive, read::ZipFile};

#[derive(Clone, Hash, PartialEq, Eq)]
struct FileDesc(SharedString, SharedString);

impl hashbrown::Equivalent<FileDesc> for (&str, &str) {
    fn equivalent(&self, key: &FileDesc) -> bool {
        key.0 == self.0 && key.1 == self.1
    }
}

/// An entry in a archive directory.
enum OwnedEntry {
    File(FileDesc),
    Dir(SharedString),
}

impl fmt::Debug for OwnedEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(FileDesc(id, ext)) => f.debug_tuple("File").field(id).field(ext).finish(),
            Self::Dir(id) => f.debug_tuple("Directory").field(id).finish(),
        }
    }
}

impl OwnedEntry {
    fn as_dir_entry(&self) -> DirEntry {
        match self {
            OwnedEntry::File(FileDesc(id, ext)) => DirEntry::File(id, ext),
            OwnedEntry::Dir(id) => DirEntry::Directory(id),
        }
    }
}

/// Register a file of an archive in maps.
fn register_file(
    file: ZipFile,
    index: usize,
    files: &mut HashMap<FileDesc, usize>,
    dirs: &mut HashMap<SharedString, Vec<OwnedEntry>>,
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
        for comp in path.parent()?.components() {
            match comp {
                path::Component::Normal(s) => id_builder.push(s.to_str()?)?,
                path::Component::ParentDir => id_builder.pop()?,
                path::Component::CurDir => continue,
                _ => return None,
            }
        }

        // Build the ids of the file and its parent.
        let parent_id = id_builder.join();
        id_builder.push(path.file_stem()?.to_str()?)?;
        let id = id_builder.join();

        // Register the file in the maps.
        let entry = if file.is_file() {
            let ext = extension_of(path)?.into();
            let desc = FileDesc(id, ext);
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
    dirs: HashMap<SharedString, Vec<OwnedEntry>>,
    archive: ZipArchive<R>,
    label: Option<String>,
}

impl Zip<SyncFile> {
    /// Creates a `Zip` archive backed by the file at the given path.
    #[inline]
    pub fn open<P: AsRef<path::Path>>(path: P) -> io::Result<Self> {
        Self::_open(path.as_ref())
    }

    #[inline]
    fn _open(path: &path::Path) -> io::Result<Self> {
        let file = SyncFile::open(path)?;
        Self::from_reader_with_label(file, path.display().to_string())
    }
}

#[cfg(feature = "mmap")]
#[cfg_attr(docsrs, doc(cfg(feature = "mmap")))]
impl Zip<io::Cursor<ArcMap>> {
    /// Creates a `Zip` archive backed by the file map at the given path.
    ///
    /// # Safety
    ///
    /// See [`ArcMap::map`] for why this this function is unsafe
    #[inline]
    pub unsafe fn mmap<P: AsRef<path::Path>>(path: P) -> io::Result<Self> {
        unsafe { Self::_mmap(path.as_ref()) }
    }

    unsafe fn _mmap(path: &path::Path) -> io::Result<Self> {
        let map = unsafe { ArcMap::map(&std::fs::File::open(path)?)? };
        let label = path.display().to_string();
        Self::from_bytes_with_label(map, label)
    }
}

impl<T: AsRef<[u8]>> Zip<io::Cursor<T>> {
    /// Creates a `Zip` archive backed by a byte buffer in memory.
    #[inline]
    pub fn from_bytes(bytes: T) -> io::Result<Self> {
        Self::from_reader(io::Cursor::new(bytes))
    }

    /// Creates a `Zip` archive backed by a byte buffer in memory.
    ///
    /// An additionnal label that will be used in errors can be added.
    #[inline]
    pub fn from_bytes_with_label(bytes: T, label: String) -> io::Result<Self> {
        Self::from_reader_with_label(io::Cursor::new(bytes), label)
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
            let file = archive.by_index_raw(index)?;
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
        let index = *self
            .files
            .get(&(id, ext))
            .ok_or_else(|| error::find_file(id, &self.label))?;
        let mut archive = self.archive.clone();
        let mut file = archive
            .by_index(index)
            .map_err(|err| error::open_file(err, id, &self.label))?;

        // Read it in a buffer
        let mut content = Vec::with_capacity(file.size() as usize);
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
            DirEntry::File(id, ext) => self.files.contains_key(&(id, ext)),
            DirEntry::Directory(id) => self.dirs.contains_key(id),
        }
    }
}

impl<R> fmt::Debug for Zip<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Zip")
            .field("dirs", &self.dirs)
            .field("label", &self.label)
            .finish()
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
