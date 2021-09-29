use std::{fmt, io};

/// A boxed error
pub type BoxedError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// The error type which is used when loading an asset.
#[derive(Debug)]
pub enum Error {
    /// An asset without extension was loaded.
    NoDefaultValue,

    /// An I/O error occured.
    Io(io::Error),

    /// The conversion from raw bytes failed.
    Conversion(BoxedError),
}

impl Error {
    pub(crate) fn or(self, other: Self) -> Self {
        use Error::*;

        match (self, other) {
            (NoDefaultValue, other) => other,
            (Io(_), other @ Conversion(_)) => other,
            (this, _) => this,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => f.pad("I/O error"),
            Self::Conversion(_) => f.pad("conversion error"),
            Self::NoDefaultValue => f.pad("no default value provided"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Conversion(err) => Some(&**err),
            Self::NoDefaultValue => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<BoxedError> for Error {
    fn from(err: BoxedError) -> Self {
        Self::Conversion(err)
    }
}
