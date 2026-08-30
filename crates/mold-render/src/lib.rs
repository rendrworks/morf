//! Backend-independent draw lists, damage tracking, and GPU instance data.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;
use std::ops::Range;

use mold_layout::{Geometry, Layout, TextAlignment, TextElide, Transform2D, node_transform};
use mold_scene::{Color, Element, NodeHandle, Scene, SceneError, Value};

mod gpu;
mod path;

pub use gpu::{GpuError, GpuInfo, WgpuBackend};

include!("commands.rs");
include!("damage.rs");
include!("sdf.rs");
include!("field.rs");
include!("paint.rs");
include!("paint_fields.rs");
include!("effects.rs");
#[cfg(test)]
mod tests;
