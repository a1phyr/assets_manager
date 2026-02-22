#[cfg(feature = "mmap")]
use super::Mmap;
use super::{DirEntry, FileContent, Source};
use crate::{
    SharedString,
    utils::{HashMap, IdBuilder, split_file_name},
};
use eazip::read::{Metadata, RawArchive};
use std::{fmt, io, path};
use sync_file::SyncFile;

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
    fn as_dir_entry(&self) -> DirEntry<'_> {
        match self {
            OwnedEntry::File(FileDesc(id, ext)) => DirEntry::File(id, ext),
            OwnedEntry::Dir(id) => DirEntry::Directory(id),
        }
    }
}

/// Register a file of an archive in maps.
fn register_file(
    index: usize,
    file: &Metadata,
    files: &mut HashMap<FileDesc, usize>,
    dirs: &mut HashMap<SharedString, Vec<OwnedEntry>>,
    id_builder: &mut IdBuilder,
) {
    id_builder.reset();

    // Check the path.
    let path = path::Path::new(file.name());

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

        let (name, ext) = split_file_name(path)?;

        // Build the ids of the file and its parent.
        let parent_id = id_builder.join();
        id_builder.push(name)?;
        let id = id_builder.join();

        // Register the file in the maps.
        let entry = match file.file_type {
            eazip::FileType::File => {
                if file.is_encrypted() {
                    log::warn!("Skipping encrypted file: {}", file.name());
                    return None;
                }

                let desc = FileDesc(id, ext.into());
                files.insert(desc.clone(), index);
                OwnedEntry::File(desc)
            }
            eazip::FileType::Directory => {
                dirs.entry(id.clone()).or_default();
                OwnedEntry::Dir(id)
            }
            eazip::FileType::Symlink => {
                log::warn!("Symlink are not supported: {}", file.name());
                return None;
            }
        };
        dirs.entry(parent_id).or_default().push(entry);

        Some(())
    })()
    .is_some();

    if !ok {
        log::warn!("Unsupported path in zip archive: {path:?}");
    }
}

type FileReader<R> = for<'a> fn(&'a R, &Metadata) -> io::Result<FileContent<'a>>;

/// A [`Source`] to load assets from a zip archive.
///
/// The archive can be backed by any reader that also implements [`io::Seek`]
/// and [`Clone`] or by a byte slice. In the second case, reading files will
/// not involve copying uncompressed data.
#[cfg_attr(docsrs, doc(cfg(feature = "zip")))]
pub struct Zip<R = SyncFile> {
    reader: R,
    read_file: FileReader<R>,

    archive: RawArchive,
    files: HashMap<FileDesc, usize>,
    dirs: HashMap<SharedString, Vec<OwnedEntry>>,
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
impl Zip<io::Cursor<Mmap>> {
    /// Creates a `Zip` archive backed by the file map at the given path.
    ///
    /// # Safety
    ///
    /// See [`Mmap::map`] for why this this function is unsafe
    #[inline]
    pub unsafe fn mmap<P: AsRef<path::Path>>(path: P) -> io::Result<Self> {
        unsafe { Self::_mmap(path.as_ref()) }
    }

    unsafe fn _mmap(path: &path::Path) -> io::Result<Self> {
        let map = unsafe { Mmap::map(&std::fs::File::open(path)?)? };
        let label = path.display().to_string();
        Self::from_bytes_with_label(map, label)
    }
}

impl<T: AsRef<[u8]>> Zip<io::Cursor<T>> {
    /// Creates a `Zip` archive backed by a byte buffer in memory.
    #[inline]
    pub fn from_bytes(bytes: T) -> io::Result<Self> {
        Self::create(io::Cursor::new(bytes), read_file_bytes::<T>, None)
    }

    /// Creates a `Zip` archive backed by a byte buffer in memory.
    ///
    /// An additionnal label that will be used in errors can be added.
    #[inline]
    pub fn from_bytes_with_label(bytes: T, label: String) -> io::Result<Self> {
        Self::create(io::Cursor::new(bytes), read_file_bytes::<T>, Some(label))
    }
}

impl<R> Zip<R>
where
    R: io::Read + io::Seek + Clone,
{
    /// Creates a `Zip` archive backed by a reader that supports seeking.
    ///
    /// **Warning**: This will clone the reader each time a file is read, so you
    /// should ensure that cloning is cheap.
    pub fn from_reader(reader: R) -> io::Result<Zip<R>> {
        Self::create(reader, read_file_reader::<R>, None)
    }

    /// Creates a `Zip` archive backed by a reader that supports seeking.
    ///
    /// An additionnal label that will be used in errors can be added.
    ///
    /// **Warning**: This will clone the reader each time a file is read, so you
    /// should ensure that cloning is cheap.
    pub fn from_reader_with_label(reader: R, label: String) -> io::Result<Zip<R>> {
        Self::create(reader, read_file_reader::<R>, Some(label))
    }
}

impl<R: io::Read + io::Seek> Zip<R> {
    fn create(
        mut reader: R,
        read_file: FileReader<R>,
        label: Option<String>,
    ) -> io::Result<Zip<R>> {
        let (archive, files, dirs) = read_archive(&mut reader)?;

        Ok(Zip {
            reader,
            read_file,

            archive,
            files,
            dirs,
            label,
        })
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "zip")))]
impl<R> Source for Zip<R>
where
    R: io::Read + io::Seek,
{
    fn read(&self, id: &str, ext: &str) -> io::Result<FileContent<'_>> {
        let index = *self
            .files
            .get(&(id, ext))
            .ok_or_else(|| error::find_file(id, &self.label))?;

        let meta = self
            .archive
            .entries()
            .get(index)
            .ok_or_else(|| error::find_file(id, &self.label))?;

        (self.read_file)(&self.reader, meta).map_err(|err| error::read_file(err, id, &self.label))
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

trait ReadSeek: io::Read + io::Seek {}
impl<R: io::Read + io::Seek> ReadSeek for R {}

trait BufReadSeek: io::BufRead + io::Seek {}
impl<R: io::BufRead + io::Seek> BufReadSeek for R {}

#[expect(clippy::type_complexity)]
fn read_archive(
    mut reader: &mut dyn ReadSeek,
) -> io::Result<(
    RawArchive,
    HashMap<FileDesc, usize>,
    HashMap<SharedString, Vec<OwnedEntry>>,
)> {
    let archive = RawArchive::new(&mut reader)?;

    let len = archive.entries().len();
    let mut files = HashMap::with_capacity(len);
    let mut dirs = HashMap::new();
    let mut id_builder = IdBuilder::default();

    for (index, file) in archive.entries().iter().enumerate() {
        register_file(index, file, &mut files, &mut dirs, &mut id_builder);
    }

    Ok((archive, files, dirs))
}

fn read_file_reader<'a, R: io::Read + io::Seek + Clone>(
    reader: &'a R,
    meta: &Metadata,
) -> io::Result<FileContent<'a>> {
    /// Polymorphisation of `read_file_reader`
    fn inner(reader: &mut dyn ReadSeek, meta: &Metadata) -> io::Result<FileContent<'static>> {
        read_file_bufreader(&mut io::BufReader::new(reader), meta)
    }

    inner(&mut reader.clone(), meta)
}

fn read_file_bufreader(
    reader: &mut dyn BufReadSeek,
    file: &Metadata,
) -> io::Result<FileContent<'static>> {
    use io::Read;

    let mut file = file.read(reader)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    Ok(FileContent::Buffer(buf))
}

fn read_file_bytes<'a, T: AsRef<[u8]>>(
    reader: &'a io::Cursor<T>,
    meta: &Metadata,
) -> io::Result<FileContent<'a>> {
    read_file_bytes_impl(reader.get_ref().as_ref(), meta)
}

/// Polymorphisation of `read_file_bytes`
fn read_file_bytes_impl<'a>(zip: &'a [u8], meta: &Metadata) -> io::Result<FileContent<'a>> {
    if meta.compression_method != eazip::CompressionMethod::STORE {
        return read_file_bufreader(&mut io::Cursor::new(zip), meta);
    }

    let start = meta.data_offset as usize;
    let file = zip
        .get(start..start + meta.compressed_size as usize)
        .ok_or(io::ErrorKind::InvalidData)?;

    if crc32fast::hash(file) != meta.crc32 {
        return Err(error::invalid_crc());
    }

    Ok(FileContent::Slice(file))
}

mod error {
    use std::{fmt, io};

    #[cold]
    pub fn invalid_crc() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, "invalid crc")
    }

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
    pub fn find_dir(id: &str, label: &Option<String>) -> io::Error {
        let msg = match label {
            Some(lbl) => format!("Could not find directory \"{id}\" in {lbl}"),
            None => format!("Could not find directory \"{id}\" in ZIP"),
        };

        io::Error::new(io::ErrorKind::NotFound, msg)
    }
}
