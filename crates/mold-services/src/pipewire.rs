pub(crate) const NODE_INTERFACE: &[u8] = b"PipeWire:Interface:Node\0";
pub(crate) const NODE_VERSION: u32 = 3;
pub(crate) const PARAM_PROPS: u32 = 2;
pub(crate) const TYPE_BOOL: u32 = 2;
pub(crate) const TYPE_FLOAT: u32 = 6;
pub(crate) const TYPE_ARRAY: u32 = 13;
pub(crate) const TYPE_OBJECT: u32 = 15;
pub(crate) const TYPE_OBJECT_PROPS: u32 = 0x40002;
pub(crate) const PROP_VOLUME: u32 = 0x10003;
pub(crate) const PROP_MUTE: u32 = 0x10004;
pub(crate) const PROP_CHANNEL_VOLUMES: u32 = 0x10008;

mod ffi;
mod runtime;
mod volume;

pub use ffi::*;
pub use runtime::*;
#[cfg(test)]
mod tests;
