//! Scene graph, typed properties, and animation targets for mold.

use std::collections::{BTreeMap, HashMap};
use std::error::Error as StdError;
use std::fmt;
use std::time::Duration;

use mold_reactive::{Graph, GraphError, SignalId};
use slotmap::{SlotMap, new_key_type};

mod model;

pub use model::{
    FlickState, ListChange, ListModel, ModelId, ViewItem, ViewTransition, VirtualList,
};

include!("types.rs");
include!("animation.rs");
include!("scene_default.rs");
include!("scene.rs");
include!("scene_access.rs");
include!("motion.rs");
include!("schema.rs");
#[cfg(test)]
mod tests;
