//! TODO

#![allow(missing_docs)]

use crate::SharedString;
use std::{
    borrow::{Borrow, Cow},
    fmt, hash,
};

/// TODO
#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Id(str);

impl ToOwned for Id {
    type Owned = OwnedId;

    #[inline]
    fn to_owned(&self) -> OwnedId {
        OwnedId(self.0.into())
    }
}

impl hash::Hash for Id {
    #[inline]
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Id {
    /// TODO
    pub const ROOT: &'static Self = Self::unchecked("");

    #[inline]
    const fn unchecked(s: &str) -> &Id {
        unsafe { &*(s as *const str as *const Id) }
    }

    /// TODO
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// TODO
    pub fn from_str(s: &str) -> Option<&Id> {
        if s.starts_with('.') || s.ends_with('.') || s.contains("..") {
            None
        } else {
            Some(Self::unchecked(s))
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns the parent's id, or `None` if it is the root.
    ///
    /// # Example
    ///
    /// ```
    /// use assets_manager::Id;
    ///
    /// let id = Id::new("example.hello.world");
    /// assert_eq!(id.parent(), Some("example.hello"));
    ///
    /// let root = Id::new("");
    /// assert!(root.parent().is_none());
    /// ```
    #[inline]
    pub fn parent(&self) -> Option<&Id> {
        if self.is_root() {
            None
        } else {
            Some(match self.0.rfind('.') {
                None => Self::ROOT,
                Some(i) => Self::unchecked(&self.0[..i]),
            })
        }
    }

    /// TODO
    #[inline]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// TODO
    #[inline]
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('.')
    }

    /// TODO
    #[inline]
    pub fn ancestors(&self) -> impl Iterator<Item = &Id> {
        let mut next = Some(self);
        std::iter::from_fn(move || {
            let res = next;
            next = next.and_then(Id::parent);
            res
        })
    }
}

impl PartialEq<str> for Id {
    fn eq(&self, other: &str) -> bool {
        self.0.eq(other)
    }
}

impl PartialEq<str> for OwnedId {
    fn eq(&self, other: &str) -> bool {
        self.0.eq(other)
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// TODO
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OwnedId(SharedString);

impl OwnedId {
    #[inline]
    pub fn from_str(s: &str) -> Cow<Id> {
        s.as_id()
    }

    /// TODO
    #[inline]
    pub fn into_shared_string(self) -> SharedString {
        self.0
    }
}

impl hash::Hash for OwnedId {
    #[inline]
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}

impl std::ops::Deref for OwnedId {
    type Target = Id;

    #[inline]
    fn deref(&self) -> &Id {
        Id::unchecked(&self.0)
    }
}

impl AsRef<Id> for OwnedId {
    #[inline]
    fn as_ref(&self) -> &Id {
        self
    }
}

impl Borrow<Id> for OwnedId {
    #[inline]
    fn borrow(&self) -> &Id {
        self
    }
}

impl fmt::Display for OwnedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

trait AsId {
    fn as_id(&self) -> Cow<Id>;
}

impl AsId for str {
    fn as_id(&self) -> Cow<Id> {
        let trimmed = self.trim_matches('.');
        if trimmed.contains("..") {
            Cow::Owned(collect_id(trimmed))
        } else {
            Cow::Borrowed(Id::unchecked(trimmed))
        }
    }
}

// /// TODO
// pub trait IdTrait {
//     /// TODO
//     fn into_owned_id(self) -> OwnedId;

//     /// TODO
//     fn to_id(&self) -> Cow<Id>;
// }

// impl IdTrait for &'_ SharedString {
//     fn into_owned_id(self) -> OwnedId {
//         if !self.starts_with('.') && self.ends_with('.') {
//             if self.contains("..") {
//                 collect_id(&self)
//             } else {
//                 OwnedId(self.clone())
//             }
//         } else {
//             let trimmed = self.trim_matches('.');
//             if trimmed.contains("..") {
//                 collect_id(trimmed)
//             } else {
//                 OwnedId(trimmed.into())
//             }
//         }
//     }

//     fn to_id(&self) -> Cow<Id> {
//         self.as_str().as_id()
//     }
// }

// impl IdTrait for SharedString {
//     fn into_owned_id(self) -> OwnedId {
//         if !self.starts_with('.') && self.ends_with('.') {
//             if self.contains("..") {
//                 collect_id(&self)
//             } else {
//                 OwnedId(self)
//             }
//         } else {
//             let trimmed = self.trim_matches('.');
//             if trimmed.contains("..") {
//                 collect_id(trimmed)
//             } else {
//                 OwnedId(trimmed.into())
//             }
//         }
//     }

//     fn to_id(&self) -> Cow<Id> {
//         self.as_str().as_id()
//     }
// }

// impl<S: AsRef<str>> IdTrait for &'_ [S] {
//     fn into_owned_id(self) -> OwnedId {
//         self.to_id().into_owned()
//     }

//     fn to_id(&self) -> Cow<Id> {
//         slice_to_id(self)
//     }
// }

// impl<S: AsRef<str>, const N: usize> IdTrait for &'_ [S; N] {
//     fn into_owned_id(self) -> OwnedId {
//         self.as_slice().into_owned_id()
//     }

//     fn to_id(&self) -> Cow<Id> {
//         slice_to_id(self.as_slice())
//     }
// }

// fn slice_to_id<S: AsRef<str>>(this: &[S]) -> Cow<Id> {
//     match this {
//         [] => Cow::Borrowed(Id::ROOT),
//         [s] => s.as_ref().as_id(),
//         _ => {
//             let estimate = this.iter().map(|s| s.as_ref().len()).sum();
//             let mut s = String::with_capacity(estimate);
//             collect_id_into(&mut s, split_slice_parts(this));
//             Cow::Owned(OwnedId(s.into()))
//         }
//     }
// }

fn collect_id_into<'a>(s: &mut String, it: impl Iterator<Item = &'a str>) {
    let mut push_dot = false;
    it.for_each(|part| {
        if push_dot {
            s.push('.');
        }
        s.push_str(part);
        push_dot = true;
    })
}

fn split_slice_parts<S: AsRef<str>>(s: &[S]) -> impl Iterator<Item = &str> {
    s.iter().flat_map(|s| s.as_ref().split('.'))
}

#[cold]
fn collect_id(id: &str) -> OwnedId {
    let mut s = String::with_capacity(id.len());
    collect_id_into(&mut s, id.split('.').filter(|p| !p.is_empty()));
    OwnedId(s.into())
}

pub trait IntoId: Sized {
    fn into_id<'a>(self) -> Cow<'a, Id>
    where
        Self: 'a;

    fn into_owned_id(self) -> OwnedId {
        self.into_id().into_owned()
    }
}

impl IntoId for &'_ str {
    fn into_id<'a>(self) -> Cow<'a, Id>
    where
        Self: 'a,
    {
        let trimmed = self.trim_matches('.');
        if trimmed.contains("..") {
            Cow::Owned(collect_id(trimmed))
        } else {
            Cow::Borrowed(Id::unchecked(trimmed))
        }
    }
}

impl IntoId for String {
    fn into_id<'a>(mut self) -> Cow<'a, Id>
    where
        Self: 'a,
    {
        // We can avoid allocating a new string here. Remove starting and
        // multiple dots.
        let mut keep_next_dot = false;
        self.retain(|c| {
            let not_dot = c == '.';
            let keep = not_dot | keep_next_dot;
            keep_next_dot = not_dot;
            keep
        });

        // There might still be a trailing dot
        if self.ends_with('.') {
            self.pop();
        }

        Cow::Owned(OwnedId(self.into()))
    }
}

impl IntoId for &'_ String {
    fn into_id<'a>(self) -> Cow<'a, Id>
    where
        Self: 'a,
    {
        self.as_str().into_id()
    }
}

impl IntoId for Cow<'_, str> {
    fn into_id<'a>(self) -> Cow<'a, Id>
    where
        Self: 'a,
    {
        match self {
            Cow::Borrowed(s) => s.into_id(),
            Cow::Owned(s) => s.into_id(),
        }
    }
}

impl IntoId for &'_ Id {
    fn into_id<'a>(self) -> Cow<'a, Id>
    where
        Self: 'a,
    {
        Cow::Borrowed(self)
    }
}

impl IntoId for OwnedId {
    fn into_id<'a>(self) -> Cow<'a, Id>
    where
        Self: 'a,
    {
        Cow::Owned(self)
    }
}

impl IntoId for &'_ OwnedId {
    fn into_id<'a>(self) -> Cow<'a, Id>
    where
        Self: 'a,
    {
        Cow::Owned(self.clone())
    }
}

impl IntoId for Cow<'_, Id> {
    fn into_id<'a>(self) -> Cow<'a, Id>
    where
        Self: 'a,
    {
        self.clone()
    }
}

impl From<OwnedId> for Cow<'_, Id> {
    fn from(id: OwnedId) -> Self {
        Cow::Owned(id)
    }
}

impl<'a> From<&'a Id> for Cow<'a, Id> {
    fn from(id: &'a Id) -> Self {
        Cow::Borrowed(id)
    }
}

impl From<&'_ Id> for OwnedId {
    fn from(id: &Id) -> Self {
        id.to_owned()
    }
}
