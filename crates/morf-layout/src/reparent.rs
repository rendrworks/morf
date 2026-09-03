//! Moving a node to a new parent without a jump.
//!
//! Split from `layout` at the line gate. Two layouts, before and after the
//! reparent, and the difference between where the node was and where it
//! will be is driven to zero through `transition_x` and `transition_y`.

use std::collections::BTreeMap;

use morf_scene::{Behavior, NodeHandle, Scene, Value};

use crate::geometry::{Size, TextMeasurer};
use crate::helpers::LayoutError;
use crate::layout::Layout;

/// Inputs for an animated parent and anchor change.
pub struct ReparentTransition {
    pub root: NodeHandle,
    pub node: NodeHandle,
    pub new_parent: NodeHandle,
    pub anchors: Option<BTreeMap<String, Value>>,
    pub available: Size,
    pub behavior: Behavior,
}

impl Layout {
    /// Reparents a node and animates its resolved position through a shared coordinate space.
    pub fn transition_reparent(
        scene: &mut Scene,
        text: &mut impl TextMeasurer,
        transition: ReparentTransition,
    ) -> Result<Self, LayoutError> {
        let before = Self::compute(scene, transition.root, transition.available, text)?
            .geometry(transition.node)
            .ok_or_else(|| LayoutError::Scene("transition node has no geometry".into()))?;
        scene.assign(transition.node, "transition_x", 0.0)?;
        scene.assign(transition.node, "transition_y", 0.0)?;
        if let Some(anchors) = transition.anchors {
            scene.assign(transition.node, "anchors", Value::Map(anchors))?;
        }
        if scene.parent(transition.node)? != Some(transition.new_parent) {
            scene.reparent(transition.node, Some(transition.new_parent))?;
        }
        let target = Self::compute(scene, transition.root, transition.available, text)?
            .geometry(transition.node)
            .ok_or_else(|| LayoutError::Scene("transition target has no geometry".into()))?;
        scene.animate_from(
            transition.node,
            "transition_x",
            before.x - target.x,
            0.0,
            transition.behavior,
        )?;
        scene.animate_from(
            transition.node,
            "transition_y",
            before.y - target.y,
            0.0,
            transition.behavior,
        )?;
        Self::compute(scene, transition.root, transition.available, text)
    }
}
