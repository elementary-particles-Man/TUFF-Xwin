pub mod args;
pub mod client;
pub mod codec;
pub mod core;
pub mod data_device;
pub mod fd;
pub mod fractional_scale;
pub mod generated;
pub mod ime_backend;
pub mod input;
pub mod input_method;
pub mod output;
pub mod presentation;
pub mod protocol;
pub mod registry;
pub mod server;
pub mod shm;
pub mod signature;
pub mod subsurface;
pub mod surface;
pub mod text_input;
pub mod viewport;
pub mod xdg_decoration;
pub mod xdg_shell;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use args::{FakeFd, WireArg};
pub use fd::WireOwnedFd;

#[derive(Debug, Error)]
pub enum WireError {
    #[error("Incomplete message (need more bytes)")]
    Incomplete,
    #[error("Invalid message size: {0}")]
    InvalidSize(u32),
    #[error("Invalid object ID: {0}")]
    InvalidObjectId(u32),
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Connection closed")]
    ConnectionClosed,
}

pub type Result<T> = std::result::Result<T, WireError>;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct WaylandObjectId(pub u32);

impl WaylandObjectId {
    pub const DISPLAY: Self = WaylandObjectId(1);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WaylandOpcode(pub u16);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaylandHeader {
    pub object_id: WaylandObjectId,
    pub size: u16,
    pub opcode: WaylandOpcode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaylandMessage {
    pub header: WaylandHeader,
    pub payload: Vec<u8>,
}

impl WaylandMessage {
    pub fn new(object_id: WaylandObjectId, opcode: WaylandOpcode, payload: Vec<u8>) -> Self {
        let size = (payload.len() + 8) as u16;
        Self { header: WaylandHeader { object_id, size, opcode }, payload }
    }
}
