use super::SharedBytes;

use std::{borrow::Cow, cmp, fmt, ops::Deref, str};

/// A string that can easily be shared.
///
/// This structure is essentially a better alternative to an `Arc<String>` or an
/// `Arc<str>`.
#[derive(Clone)]
pub struct SharedString {
    // Safety: must be valid UTF-8
    bytes: SharedBytes,
}

impl SharedString {
    /// Converts a `SharedBytes` to a `SharedString`.
    ///
    /// Returns `Err` if `bytes` does not contain valid UTF-8.
    #[inline]
    pub fn from_utf8(bytes: SharedBytes) -> Result<SharedString, str::Utf8Error> {
        let _ = str::from_utf8(&bytes)?;
        Ok(SharedString { bytes })
    }

    /// Converts a `SharedBytes` to a `SharedString`, without checking that the
    /// string contains valid UTF-8.
    ///
    /// # Safety
    ///
    /// `bytes` must contain valid UTF-8.
    #[inline]
    pub unsafe fn from_utf8_unchecked(bytes: SharedBytes) -> SharedString {
        SharedString { bytes }
    }

    #[inline]
    pub(crate) fn n_from_str<const N: usize>(s: &str) -> [Self; N] {
        SharedBytes::n_from_slice(s.as_bytes()).map(|bytes| Self { bytes })
    }

    /// Converts the `&SharedString` into a `&str`.
    #[inline]
    pub fn as_str(&self) -> &str {
        self
    }

    /// Converts the `&SharedString` into a `String`.
    #[inline]
    #[allow(clippy::inherent_to_string_shadow_display)]
    pub fn to_string(&self) -> String {
        String::from(&**self)
    }

    /// Converts the `SharedString` into `SharedBytes`.
    ///
    /// This methods does not allocate nor copies memory.
    #[inline]
    pub fn into_bytes(self) -> SharedBytes {
        self.bytes
    }
}

impl Deref for SharedString {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        unsafe { str::from_utf8_unchecked(&self.bytes) }
    }
}

impl AsRef<str> for SharedString {
    #[inline]
    fn as_ref(&self) -> &str {
        self
    }
}

impl AsRef<[u8]> for SharedString {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl AsRef<std::path::Path> for SharedString {
    #[inline]
    fn as_ref(&self) -> &std::path::Path {
        (**self).as_ref()
    }
}

impl AsRef<std::ffi::OsStr> for SharedString {
    #[inline]
    fn as_ref(&self) -> &std::ffi::OsStr {
        (**self).as_ref()
    }
}

impl std::borrow::Borrow<str> for SharedString {
    #[inline]
    fn borrow(&self) -> &str {
        self
    }
}

impl From<String> for SharedString {
    #[inline]
    fn from(s: String) -> Self {
        let bytes = SharedBytes::from_vec(s.into_bytes());
        SharedString { bytes }
    }
}

impl From<&str> for SharedString {
    #[inline]
    fn from(s: &str) -> Self {
        let bytes = SharedBytes::from_slice(s.as_bytes());
        SharedString { bytes }
    }
}

impl From<Cow<'_, str>> for SharedString {
    #[inline]
    fn from(s: Cow<str>) -> Self {
        match s {
            Cow::Owned(s) => SharedString::from(s),
            Cow::Borrowed(s) => SharedString::from(s),
        }
    }
}

impl std::hash::Hash for SharedString {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        (**self).hash(hasher);
    }
}

impl PartialEq<str> for SharedString {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        **self == *other
    }
}

impl PartialEq<&str> for SharedString {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        **self == **other
    }
}

impl PartialEq<String> for SharedString {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        **self == *other
    }
}

impl PartialEq for SharedString {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for SharedString {}

impl PartialOrd<str> for SharedString {
    fn partial_cmp(&self, other: &str) -> Option<cmp::Ordering> {
        (**self).partial_cmp(other)
    }
}

impl PartialOrd for SharedString {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        (**self).partial_cmp(&**other)
    }
}

impl Ord for SharedString {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        (**self).cmp(other)
    }
}

impl fmt::Debug for SharedString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl fmt::Display for SharedString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl serde::Serialize for SharedString {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self)
    }
}

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl<'de> serde::Deserialize<'de> for SharedString {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = SharedString;

            #[inline]
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string")
            }

            #[inline]
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Self::Value, E> {
                Ok(SharedString::from(s))
            }

            #[inline]
            fn visit_string<E: serde::de::Error>(self, s: String) -> Result<Self::Value, E> {
                Ok(SharedString::from(s))
            }

            #[inline]
            fn visit_bytes<E: serde::de::Error>(self, s: &[u8]) -> Result<Self::Value, E> {
                match str::from_utf8(s) {
                    Ok(s) => Ok(SharedString::from(s)),
                    Err(_) => {
                        let unexp = serde::de::Unexpected::Bytes(s);
                        Err(E::invalid_value(unexp, &self))
                    }
                }
            }

            #[inline]
            fn visit_byte_buf<E: serde::de::Error>(self, s: Vec<u8>) -> Result<Self::Value, E> {
                match String::from_utf8(s) {
                    Ok(s) => Ok(SharedString::from(s)),
                    Err(e) => {
                        let unexp = serde::de::Unexpected::Bytes(e.as_bytes());
                        Err(E::invalid_value(unexp, &self))
                    }
                }
            }
        }

        deserializer.deserialize_string(Visitor)
    }
}
