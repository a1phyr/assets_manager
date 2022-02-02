//! Stub implementation of the module's API

#![allow(missing_docs)]

pub use crate::key::{AssetKey, AssetType};
use crate::BoxedError;

#[derive(Debug, Clone)]
enum Void {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateMessage {
    AddAsset(AssetKey),
    RemoveAsset(AssetKey),
    Clear,
}

#[derive(Debug)]
pub struct Disconnected;

#[derive(Debug, Clone)]
pub struct EventSender(Void);
impl EventSender {
    pub fn send(&self, _: AssetKey) -> Result<(), Disconnected> {
        match self.0 {}
    }
}

pub trait UpdateSender {
    fn send_update(&self, update: UpdateMessage);
}

pub type DynUpdateSender = Box<dyn UpdateSender + Send + Sync>;

impl<T> UpdateSender for Box<T>
where
    T: UpdateSender + ?Sized,
{
    fn send_update(&self, message: UpdateMessage) {
        (**self).send_update(message)
    }
}

impl<T> UpdateSender for std::sync::Arc<T>
where
    T: UpdateSender + ?Sized,
{
    fn send_update(&self, message: UpdateMessage) {
        (**self).send_update(message)
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

    pub fn build(self, _: EventSender) -> DynUpdateSender {
        match self.0 {}
    }
}
