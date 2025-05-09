use std::{fmt, io};

use crate::SharedString;

/// A boxed error
pub type BoxedError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug)]
pub(crate) enum ErrorKind {
    /// An asset without extension was loaded.
    NoDefaultValue,

    /// An I/O error occured.
    Io(io::Error),

    /// The conversion from raw bytes failed.
    Conversion(BoxedError),

    /// The provided ID was invalid.
    InvalidId,
}

impl From<io::Error> for ErrorKind {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<BoxedError> for ErrorKind {
    fn from(err: BoxedError) -> Self {
        Self::Conversion(err)
    }
}

impl From<ErrorKind> for BoxedError {
    fn from(err: ErrorKind) -> Self {
        match err {
            ErrorKind::NoDefaultValue => Box::new(NoDefaultValueError),
            ErrorKind::Io(err) => Box::new(err),
            ErrorKind::Conversion(err) => err,
            ErrorKind::InvalidId => Box::new(InvalidIdError),
        }
    }
}

impl ErrorKind {
    pub fn or(self, other: Self) -> Self {
        use ErrorKind::*;

        match (self, other) {
            (NoDefaultValue, other) => other,
            (Io(_), other @ Conversion(_)) => other,
            (Io(err), other @ Io(_)) if err.kind() == io::ErrorKind::NotFound => other,
            (this, _) => this,
        }
    }
}

#[derive(Debug)]
struct NoDefaultValueError;

impl fmt::Display for NoDefaultValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the asset has neither extension nor default value")
    }
}

impl std::error::Error for NoDefaultValueError {}

#[derive(Debug)]
struct InvalidIdError;

impl fmt::Display for InvalidIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid id")
    }
}

impl std::error::Error for InvalidIdError {}

struct ErrorRepr {
    id: SharedString,
    error: BoxedError,
}

/// The error type which is used when loading an asset.
pub struct Error(Box<ErrorRepr>);

impl Error {
    #[cold]
    pub(crate) fn new(id: SharedString, error: BoxedError) -> Self {
        Self(Box::new(ErrorRepr { id, error }))
    }

    /// The id of the asset that was being loaded when the error happened.
    #[inline]
    pub fn id(&self) -> &SharedString {
        &self.0.id
    }

    /// Like `source`, but never fails.
    #[inline]
    pub fn reason(&self) -> &(dyn std::error::Error + 'static) {
        &*self.0.error
    }

    /// Consumes the `Error`, returning its inner error.
    #[inline]
    pub fn into_inner(self) -> BoxedError {
        self.0.error
    }

    /// Attempt to downgrade the inner error to `E`.
    #[inline]
    pub fn downcast<E: std::error::Error + 'static>(mut self) -> Result<E, Self> {
        match self.0.error.downcast() {
            Ok(err) => Ok(*err),
            Err(err) => {
                self.0.error = err;
                Err(self)
            }
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("id", &self.0.id)
            .field("error", &self.0.error)
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("failed to load \"{}\"", self.id()))
    }
}

impl std::error::Error for Error {
    #[inline]
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.reason())
    }
}
