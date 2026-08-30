//! Three-stage layout for mold scene nodes.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::error::Error as StdError;
use std::fmt;
use std::hash::{Hash, Hasher};

use mold_scene::{Behavior, Element, NodeHandle, Scene, SceneError, Value};

include!("geometry.rs");
include!("layout.rs");
include!("hit.rs");
include!("transform.rs");
include!("helpers.rs");
#[cfg(test)]
mod tests;
