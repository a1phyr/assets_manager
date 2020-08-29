use std::{
    borrow::Cow,
    collections::HashMap,
    io,
};


/// TODO
#[cfg_attr(docsrs, doc(cfg(feature = "embedded")))]
#[derive(Clone, Copy, Debug)]
pub struct RawEmbedded<'a> {
    /// TODO
    pub files: &'a [((&'a str, &'a str), &'a [u8])],

    /// TODO
    pub dirs: &'a [(&'a str, &'a [(&'a str, &'a str)])],
}

/// TODO
#[cfg_attr(docsrs, doc(cfg(feature = "embedded")))]
#[derive(Clone, Debug)]
pub struct Embedded<'a> {
    files: HashMap<(&'a str, &'a str), &'a [u8]>,
    dirs: HashMap<&'a str, &'a [(&'a str, &'a str)]>,
}

impl<'a> From<RawEmbedded<'a>> for Embedded<'a> {
    fn from(raw: RawEmbedded<'a>) -> Embedded<'a> {
        Embedded {
            files: raw.files.iter().copied().collect(),
            dirs: raw.dirs.iter().copied().collect(),
        }
    }
}

impl<'a> super::Source for Embedded<'a> {
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>> {
        match self.files.get(&(id, ext)) {
            Some(content) => Ok(Cow::Borrowed(content)),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn read_dir(&self, dir: &str, ext: &[&str]) -> io::Result<Vec<String>> {
        let dir = self.dirs.get(dir).ok_or(io::ErrorKind::NotFound)?;

        Ok(dir.iter().copied()
            .filter(|(_, file_ext)| ext.contains(file_ext))
            .map(|(id,_)| id.to_owned())
            .collect()
        )
    }
}
