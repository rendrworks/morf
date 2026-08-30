use libloading::Library;
use std::collections::BTreeMap;
use std::env;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt;
use std::mem;
use std::path::PathBuf;
use std::ptr;
use std::sync::Mutex;

const NODE_INTERFACE: &[u8] = b"PipeWire:Interface:Node\0";
const NODE_VERSION: u32 = 3;
const PARAM_PROPS: u32 = 2;
const TYPE_BOOL: u32 = 2;
const TYPE_FLOAT: u32 = 6;
const TYPE_ARRAY: u32 = 13;
const TYPE_OBJECT: u32 = 15;
const TYPE_OBJECT_PROPS: u32 = 0x40002;
const PROP_VOLUME: u32 = 0x10003;
const PROP_MUTE: u32 = 0x10004;
const PROP_CHANNEL_VOLUMES: u32 = 0x10008;

include!("pipewire/ffi.rs");
include!("pipewire/runtime.rs");
include!("pipewire/volume.rs");
#[cfg(test)]
mod tests;
