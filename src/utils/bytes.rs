use std::{
    alloc,
    borrow::Cow,
    cmp, fmt,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Inner {
    count: AtomicUsize,
    ptr: *const u8,
    len: usize,
    capacity: usize,
}

/// Bytes that can easily be shared.
///
/// This structure is essentially a better alternative to an `Arc<Vec<u8>>`
/// when created from a slice.
pub struct SharedBytes {
    ptr: NonNull<Inner>,
}

unsafe impl Send for SharedBytes {}
unsafe impl Sync for SharedBytes {}

impl Deref for SharedBytes {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        let inner = self.inner();
        unsafe { std::slice::from_raw_parts(inner.ptr, inner.len) }
    }
}

impl Clone for SharedBytes {
    #[inline]
    fn clone(&self) -> Self {
        self.inner().count.fetch_add(1, Ordering::Relaxed);
        Self { ptr: self.ptr }
    }
}

impl Drop for SharedBytes {
    #[inline]
    fn drop(&mut self) {
        // Synchronize with `drop_slow`
        if self.inner().count.fetch_sub(1, Ordering::Release) == 1 {
            unsafe {
                self.drop_slow();
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

impl SharedBytes {
    #[inline]
    fn inner(&self) -> &Inner {
        unsafe { self.ptr.as_ref() }
    }

    #[inline]
    fn get_inner_layout(len: usize) -> alloc::Layout {
        #[cold]
        fn too_long() -> ! {
            panic!("Invalid slice length");
        }

        let slice_layout = unsafe { alloc::Layout::from_size_align_unchecked(len, 1) };
        let (layout, offset) = alloc::Layout::new::<Inner>()
            .extend(slice_layout)
            .unwrap_or_else(|_| too_long());
        debug_assert_eq!(offset, std::mem::size_of::<Inner>());
        layout
    }

    #[cold]
    unsafe fn drop_slow(&mut self) {
        let inner = self.inner();

        // Synchronize with `drop`
        inner.count.load(Ordering::Acquire);

        let layout = if inner.capacity != 0 {
            drop(Vec::from_raw_parts(
                inner.ptr as *mut u8,
                inner.len,
                inner.capacity,
            ));
            alloc::Layout::new::<Inner>()
        } else {
            Self::get_inner_layout(inner.len)
        };

        alloc::dealloc(self.ptr.as_ptr().cast(), layout);
    }

    fn inner_from_slice(bytes: &[u8], count: usize) -> NonNull<Inner> {
        unsafe {
            let len = bytes.len();
            let layout = Self::get_inner_layout(len);
            let ptr = alloc::alloc(layout).cast::<Inner>();
            let bytes_ptr = ptr.add(1).cast::<u8>();
            let ptr = NonNull::new(ptr).unwrap_or_else(|| alloc::handle_alloc_error(layout));

            ptr.as_ptr().write(Inner {
                count: AtomicUsize::new(count),
                ptr: bytes_ptr,
                len,
                capacity: 0,
            });
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_ptr, len);

            ptr
        }
    }

    #[inline]
    pub(crate) fn n_from_slice<const N: usize>(bytes: &[u8]) -> [Self; N] {
        let ptr = Self::inner_from_slice(bytes, N);
        [(); N].map(|_| Self { ptr })
    }

    /// Create a `SharedBytes` from a slice.
    #[inline]
    pub fn from_slice(bytes: &[u8]) -> Self {
        let ptr = Self::inner_from_slice(bytes, 1);
        Self { ptr }
    }

    fn inner_from_vec(bytes: Vec<u8>, count: usize) -> NonNull<Inner> {
        unsafe {
            let layout = alloc::Layout::new::<Inner>();
            let ptr = alloc::alloc(layout).cast::<Inner>();
            let ptr = NonNull::new(ptr).unwrap_or_else(|| alloc::handle_alloc_error(layout));

            let bytes = std::mem::ManuallyDrop::new(bytes);
            let bytes_ptr = bytes.as_ptr();
            let len = bytes.len();
            let capacity = bytes.capacity();
            ptr.as_ptr().write(Inner {
                count: AtomicUsize::new(count),
                ptr: bytes_ptr,
                len,
                capacity,
            });

            ptr
        }
    }

    /// Create a `SharedBytes` from a `Vec`
    #[inline]
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        let ptr = Self::inner_from_vec(bytes, 1);
        Self { ptr }
    }
}

impl From<&[u8]> for SharedBytes {
    #[inline]
    fn from(bytes: &[u8]) -> SharedBytes {
        SharedBytes::from_slice(bytes)
    }
}

impl From<Vec<u8>> for SharedBytes {
    #[inline]
    fn from(bytes: Vec<u8>) -> SharedBytes {
        SharedBytes::from_vec(bytes)
    }
}

impl From<Box<[u8]>> for SharedBytes {
    #[inline]
    fn from(bytes: Box<[u8]>) -> SharedBytes {
        SharedBytes::from_vec(bytes.into_vec())
    }
}

impl From<Cow<'_, [u8]>> for SharedBytes {
    #[inline]
    fn from(bytes: Cow<[u8]>) -> SharedBytes {
        match bytes {
            Cow::Borrowed(b) => SharedBytes::from_slice(b),
            Cow::Owned(b) => SharedBytes::from_vec(b),
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
        let bytes = iter.into_iter().collect();
        SharedBytes::from_vec(bytes)
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
        (**self).partial_cmp(&**other)
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
                Ok(SharedBytes::from_slice(v.as_bytes()))
            }

            #[inline]
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(SharedBytes::from_vec(v.into_bytes()))
            }

            #[inline]
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                Ok(SharedBytes::from_slice(v))
            }

            #[inline]
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(SharedBytes::from_vec(v))
            }
        }

        deserializer.deserialize_byte_buf(Visitor)
    }
}
