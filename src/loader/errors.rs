//! Definitions of error types

use std::{
    error::Error,
    fmt,
    io,
    str::Utf8Error,
    string::FromUtf8Error,
};

/// An error which occurs when loading a `String`.
///
/// This error is used as the error type of [`StringLoader`].
///
/// [`StringLoader`]: struct.StringLoader.html
#[derive(Debug)]
pub enum StringLoaderError {
    /// An I/O error has occured while loading the file from disk.
    Io(io::Error),

    /// The loaded file was not valid UTF-8.
    Utf8(FromUtf8Error),
}

impl From<io::Error> for StringLoaderError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<FromUtf8Error> for StringLoaderError {
    fn from(err: FromUtf8Error) -> Self {
        Self::Utf8(err)
    }
}

impl fmt::Display for StringLoaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => err.fmt(f),
            Self::Utf8(err) => err.fmt(f),
        }
    }
}

impl Error for StringLoaderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Utf8(err) => Some(err),
        }
    }
}


/// An error which occurs when loading a parsed value.
///
/// This error is used as the error type of [`ParseLoader`].
///
/// [`ParseLoader`]: struct.ParseLoader.html
#[derive(Debug)]
pub enum ParseLoaderError<E> {
    /// An I/O error occured when loading the file from disk.
    Io(io::Error),

    /// The loaded file was not valid UTF-8.
    Utf8(Utf8Error),

    /// An error occured when parsing the file.
    Parse(E),
}

impl<E> From<io::Error> for ParseLoaderError<E> {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl<E> From<Utf8Error> for ParseLoaderError<E> {
    fn from(err: Utf8Error) -> Self {
        Self::Utf8(err)
    }
}

impl<E> fmt::Display for ParseLoaderError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => err.fmt(f),
            Self::Utf8(err) => err.fmt(f),
            Self::Parse(err) => err.fmt(f),
        }
    }
}

impl<E> Error for ParseLoaderError<E>
where
    E: Error + 'static
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Utf8(err) => Some(err),
            Self::Parse(err) => Some(err),
        }
    }
}
