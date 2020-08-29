//! TODO

use std::{borrow::Cow, io};


mod filesystem;

pub use filesystem::FileSystem;


#[cfg(feature = "embedded")]
mod embedded;

#[cfg(feature = "embedded")]
pub use embedded::{Embedded, RawEmbedded};

/// TODO
#[cfg(feature = "embedded")]
#[cfg_attr(docsrs, doc(cfg(feature = "embedded")))]
pub use assets_manager_macros::embed;


#[cfg(test)]
mod tests;


/// TODO
pub trait Source {
    /// TODO
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>>;

    /// TODO
    fn read_dir(&self, dir: &str, ext: &[&str]) -> io::Result<Vec<String>>;

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_add_asset<A: crate::Asset>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_add_dir<A: crate::Asset>(&self, _: &str) where Self: Sized {}

    #[cfg(feature = "hot-reloading")]
    #[doc(hidden)]
    fn __private_hr_clear(&mut self) where Self: Sized {}
}

impl<S> Source for Box<S>
where
    S: Source + ?Sized,
{
    fn read(&self, id: &str, ext: &str) -> io::Result<Cow<[u8]>> {
        self.as_ref().read(id, ext)
    }

    fn read_dir(&self, dir: &str, ext: &[&str]) -> io::Result<Vec<String>> {
        self.as_ref().read_dir(dir, ext)
    }
}

