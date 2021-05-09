//! Various utility types

mod bytes;
pub use bytes::SharedBytes;

mod private;
pub(crate) use private::*;

#[cfg(test)]
mod tests;
