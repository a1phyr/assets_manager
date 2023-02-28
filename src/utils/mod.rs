//! Various utility types

mod bytes;
pub use bytes::SharedBytes;

mod string;
pub use string::SharedString;

mod private;
pub(crate) use private::*;

#[cfg(test)]
mod tests;

#[cfg(feature = "utils")]
pub mod cell;
