//! Definitions of error types

use std::{
    error::Error,
    fmt,
    io,
};

/// An error that occured when loading an asset.
#[derive(Debug)]
#[non_exhaustive]
pub enum AssetError {
    /// An I/O error occurred while trying to load the asset.
    IoError(io::Error),

    /// An error occurred when changing raw bytes into the asset type.
    LoadError(Box<dyn Error + Send + Sync>),
}

impl From<io::Error> for AssetError {
    fn from(err: io::Error) -> Self {
        Self::IoError(err)
    }
}

#[allow(deprecated)]
impl fmt::Display for AssetError {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                AssetError::IoError(err) => write!(f, "An I/O error occurred while trying to load an asset : {}", err),
                AssetError::LoadError(err) => write!(f, "An conversion error occurred while trying to load an asset : {}", err),
            }
     }
}

impl Error for AssetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            AssetError::IoError(err) => Some(err),
            AssetError::LoadError(err) => Some(err.as_ref()),
        }
    }
}
