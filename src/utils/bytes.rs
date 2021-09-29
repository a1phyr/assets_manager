use std::{borrow::Cow, cmp, fmt, ops::Deref, sync::Arc};

#[repr(C)]
#[derive(Clone, Copy)]
struct Raw {
    zero: usize,
    ptr: *const Vec<u8>,
}

#[repr(C)]
#[derive(Clone, Copy)]
union Inner {
    slice: *const [u8],
    vec: Raw,
}

/// Bytes that can easily be shared.
///
/// This structure is essentially a better alternative to an `Arc<Vec<u8>>`
/// when created from a slice.
pub struct SharedBytes(Inner);

unsafe impl Send for SharedBytes {}
unsafe impl Sync for SharedBytes {}

impl Deref for SharedBytes {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        unsafe {
            if let Raw { zero: 0, ptr } = self.0.vec {
                let vec: &Vec<u8> = &*ptr;
                vec
            } else {
                &*self.0.slice
            }
        }
    }
}

impl Clone for SharedBytes {
    #[inline]
    fn clone(&self) -> Self {
        unsafe {
            if let Raw { zero: 0, ptr } = self.0.vec {
                Arc::increment_strong_count(ptr);
            } else {
                Arc::increment_strong_count(self.0.slice);
            }

            SharedBytes(self.0)
        }
    }
}

impl Drop for SharedBytes {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            if let Raw { zero: 0, ptr } = self.0.vec {
                Arc::decrement_strong_count(ptr);
            } else {
                Arc::decrement_strong_count(self.0.slice);
            }
        }
    }
}

impl AsRef<[u8]> for SharedBytes {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl std::borrow::Borrow<[u8]> for SharedBytes {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl From<Arc<[u8]>> for SharedBytes {
    #[inline]
    fn from(arc: Arc<[u8]>) -> SharedBytes {
        let ptr = Arc::into_raw(arc);
        SharedBytes(Inner { slice: ptr })
    }
}

impl From<Arc<Vec<u8>>> for SharedBytes {
    #[inline]
    fn from(arc: Arc<Vec<u8>>) -> SharedBytes {
        let ptr = Arc::into_raw(arc);
        SharedBytes(Inner {
            vec: Raw { zero: 0, ptr },
        })
    }
}

impl From<&[u8]> for SharedBytes {
    #[inline]
    fn from(slice: &[u8]) -> SharedBytes {
        SharedBytes::from(<Arc<[u8]>>::from(slice))
    }
}

impl From<Vec<u8>> for SharedBytes {
    #[inline]
    fn from(vec: Vec<u8>) -> SharedBytes {
        SharedBytes::from(Arc::new(vec))
    }
}

impl From<Box<[u8]>> for SharedBytes {
    #[inline]
    fn from(vec: Box<[u8]>) -> SharedBytes {
        SharedBytes::from(Vec::from(vec))
    }
}

impl From<Cow<'_, [u8]>> for SharedBytes {
    #[inline]
    fn from(cow: Cow<[u8]>) -> SharedBytes {
        match cow {
            Cow::Borrowed(b) => SharedBytes::from(b),
            Cow::Owned(v) => SharedBytes::from(v),
        }
    }
}

impl From<&SharedBytes> for SharedBytes {
    #[inline]
    fn from(bytes: &SharedBytes) -> SharedBytes {
        bytes.clone()
    }
}

impl std::iter::FromIterator<u8> for SharedBytes {
    fn from_iter<T>(iter: T) -> SharedBytes
    where
        T: IntoIterator<Item = u8>,
    {
        let vec: Vec<_> = iter.into_iter().collect();
        SharedBytes::from(vec)
    }
}

impl std::hash::Hash for SharedBytes {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.as_ref().hash(hasher);
    }
}

impl PartialEq<[u8]> for SharedBytes {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        **self == *other
    }
}

impl PartialEq<&[u8]> for SharedBytes {
    #[inline]
    fn eq(&self, other: &&[u8]) -> bool {
        **self == **other
    }
}

impl PartialEq<Vec<u8>> for SharedBytes {
    #[inline]
    fn eq(&self, other: &Vec<u8>) -> bool {
        **self == *other
    }
}

impl PartialEq for SharedBytes {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for SharedBytes {}

impl PartialOrd<[u8]> for SharedBytes {
    fn partial_cmp(&self, other: &[u8]) -> Option<cmp::Ordering> {
        (**self).partial_cmp(other)
    }
}

impl PartialOrd for SharedBytes {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        (**self).partial_cmp(other)
    }
}

impl Ord for SharedBytes {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        (**self).cmp(other)
    }
}

impl fmt::Debug for SharedBytes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(&**self).finish()
    }
}

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl serde::Serialize for SharedBytes {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(self)
    }
}

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl<'de> serde::Deserialize<'de> for SharedBytes {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = SharedBytes;

            #[inline]
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("bytes")
            }

            #[inline]
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(SharedBytes::from(v.as_bytes()))
            }

            #[inline]
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(SharedBytes::from(v.into_bytes()))
            }

            #[inline]
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                Ok(SharedBytes::from(v))
            }

            #[inline]
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(SharedBytes::from(v))
            }
        }

        deserializer.deserialize_byte_buf(Visitor)
    }
}
