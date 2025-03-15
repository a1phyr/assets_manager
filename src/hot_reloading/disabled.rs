//! Stub implementation of the module's API

#![allow(missing_docs)]

use crate::{BoxedError, source::OwnedDirEntry};

#[derive(Debug, Clone)]
enum Void {}

#[derive(Debug)]
pub struct Disconnected;

#[derive(Debug, Clone)]
pub struct EventSender(Void);

impl EventSender {
    pub fn send(&self, _: OwnedDirEntry) -> Result<(), Disconnected> {
        match self.0 {}
    }

    pub fn send_multiple<I>(&self, _: I) -> Result<usize, Disconnected>
    where
        I: IntoIterator<Item = OwnedDirEntry>,
    {
        match self.0 {}
    }
}

#[derive(Debug)]
pub struct FsWatcherBuilder(Void);

impl FsWatcherBuilder {
    #[inline]
    pub fn new() -> Result<Self, BoxedError> {
        Err("hot-reloading feature is disabled".into())
    }

    pub fn watch(&mut self, _: std::path::PathBuf) -> Result<(), BoxedError> {
        match self.0 {}
    }

    pub fn build(self, _: EventSender) {
        match self.0 {}
    }
}

#[derive(Clone, Debug)]
pub struct Recorder(Void);

impl Recorder {
    pub fn current() -> Self {
        panic!("no recorder installed")
    }

    pub fn install<T>(&self, _: impl FnOnce() -> T) -> T {
        match self.0 {}
    }
}
