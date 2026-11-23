//! Background work: decoding, thumbnails, HTTP.

pub mod archive;
mod atomic;
pub mod ipc;
pub mod loader;
pub mod notify;
pub mod remote;
pub mod state;
pub mod thumbs;
#[cfg(feature = "video")]
pub mod video;
