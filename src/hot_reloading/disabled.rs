#![allow(missing_docs, missing_debug_implementations)]

pub use crate::key::{AssetKey, AssetType};
use crate::source::Source;

enum Void {}

pub enum UpdateMessage {
    AddAsset(AssetKey),
    Clear,
}

pub struct Disconnected;

pub struct EventSender(Void);
impl EventSender {
    pub fn send(&self, _: AssetKey) -> Result<(), Disconnected> {
        match self.0 {}
    }
}

pub enum TryRecvUpdateError {
    Disconnected,
    Empty,
}

pub struct UpdateReceiver(Void);
impl UpdateReceiver {
    pub fn recv(&self) -> Result<UpdateMessage, Disconnected> {
        match self.0 {}
    }

    pub fn try_recv(&self) -> Result<UpdateMessage, TryRecvUpdateError> {
        match self.0 {}
    }
}

pub struct HotReloaderConfig {
    _void: Void,
}

pub fn config_hot_reloading() -> (EventSender, UpdateReceiver, HotReloaderConfig) {
    panic!("Hot reloading is disabled")
}

pub struct HotReloader {
    _void: Void,
}

impl HotReloader {
    pub fn start<S: Source + Send + 'static>(config: HotReloaderConfig, _: S) -> Self {
        match config._void {}
    }
}
