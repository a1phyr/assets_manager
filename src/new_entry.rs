use std::{sync::atomic::{AtomicBool, AtomicUsize, Ordering}, cell::UnsafeCell, ptr::NonNull, marker::PhantomData};

use crate::SharedString;


struct Header {
    id: SharedString,
    weak_count: AtomicUsize,
}

#[repr(C)]
struct StaticInner<T> {
    header: Header,
    value: T,
}


#[repr(C)]
struct DynamicInner<T> {
    header: Header,
    value: UnsafeCell<T>,
    reloaded: AtomicBool,
}

pub struct CacheEntry(NonNull<Header>);

impl CacheEntry {
    pub fn new<T: 'static>(value: T) -> Self {
        todo!()
    }

    fn header(&self) -> &Header {
        unsafe {
            self.0.as_ref()
        }
    }

    #[inline]
    pub(crate) fn inner(&self) -> UntypedHandle {
        UntypedHandle {
            ptr: self.0,
            lt: PhantomData,
        }
    }

    /// Consumes the `CacheEntry` and returns its inner value.
    #[inline]
    pub fn into_inner<T: 'static>(self) -> (T, SharedString) {
        todo!()
        // let _this = match self.0.downcast::<StaticInner<T>>() {
        //     Ok(inner) => return (inner.value, inner.id),
        //     Err(this) => this,
        // };

        // #[cfg(feature = "hot-reloading")]
        // if let Ok(inner) = _this.downcast::<DynamicInner<T>>() {
        //     return (inner.value.into_inner(), inner.id);
        // }

        // wrong_handle_type()
    }
}

impl Drop for CacheEntry {
    fn drop(&mut self) {
        let header = self.header();

        if header.weak_count.load(Ordering::Relaxed) == 0 {
            // drop all
        }
    }
}

pub struct UntypedHandle<'a> {
    ptr: NonNull<Header>,
    lt: PhantomData<&'a Header>,
}


pub struct WeakHandle<T> {
    header: NonNull<Header>,
    typ: PhantomData<T>,
}

impl<T> WeakHandle<T> {
    unsafe fn from_header(header: NonNull<Header>) -> Self {
        Self { header, typ: PhantomData }
    }

    fn header(&self) -> &Header {
        unsafe {
            self.header.as_ref()
        }
    }
}

impl<T> Clone for WeakHandle<T> {
    fn clone(&self) -> Self {
        self.header().weak_count.fetch_add(1, Ordering::Relaxed);
        unsafe {Self::from_header(self.header)}
    }
}

impl<T> Drop for WeakHandle<T> {
    fn drop(&mut self) {
        if self.header().weak_count.fetch_sub(1, Ordering::Release) == 0 {
            // drop all
            return ;
        }
    }
}

pub struct Handle<'a, T> {
    header: NonNull<Header>,
    _marker: PhantomData<&'a T>,
}

impl<'a, T> Handle<'a, T> {
    fn header(&self) -> &Header {
        unsafe {
            self.header.as_ref()
        }
    }

    pub fn weak(&self) -> WeakHandle<T> {
        self.header().weak_count.fetch_add(1, Relaxed);
        WeakHandle {
            header: self.header,
            typ: PhantomData,
        }
    }
}
