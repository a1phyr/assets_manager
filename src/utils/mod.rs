//! Various utility types

mod bytes;
pub use bytes::SharedBytes;

mod string;
pub use string::SharedString;

pub use crate::dirs::Directory;

mod private;
pub(crate) use private::*;

#[cfg(test)]
mod tests;
