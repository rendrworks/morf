use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use morf_scene::{NodeHandle, Scene, Value};

use crate::geometry::{Geometry, Transform2D, TransformParameters};
use crate::helpers::LayoutError;
use crate::layout::{Layout, TransformTracker, TransformWatcher};

impl TransformTracker {
    /// Merges one rendered surface layout into the geometry cache.
    pub fn update(&mut self, layout: &Layout) {
        self.geometry.extend(layout.geometries());
    }

    /// Returns the latest resolved geometry for a scene node.
    pub fn geometry(&self, node: NodeHandle) -> Option<Geometry> {
        self.geometry.get(&node).copied()
    }

    /// Maps a point from node-local coordinates into surface coordinates.
    pub fn map_from_node(
        &self,
        scene: &Scene,
        node: NodeHandle,
        x: f64,
        y: f64,
    ) -> Result<Option<(f64, f64)>, LayoutError> {
        let Some(geometry) = self.geometry(node) else {
            return Ok(None);
        };
        let Some(transform) = self.node_to_surface_transform(scene, node)? else {
            return Ok(None);
        };
        Ok(Some(transform.point(geometry.x + x, geometry.y + y)))
    }

    /// Maps a node-local rectangle into surface-aligned bounds.
    pub fn map_rect_from_node(
        &self,
        scene: &Scene,
        node: NodeHandle,
        geometry: Geometry,
    ) -> Result<Option<Geometry>, LayoutError> {
        let Some(node_geometry) = self.geometry(node) else {
            return Ok(None);
        };
        let Some(transform) = self.node_to_surface_transform(scene, node)? else {
            return Ok(None);
        };
        Ok(Some(transform.bounds(Geometry {
            x: node_geometry.x + geometry.x,
            y: node_geometry.y + geometry.y,
            width: geometry.width,
            height: geometry.height,
        })))
    }

    fn node_to_surface_transform(
        &self,
        scene: &Scene,
        node: NodeHandle,
    ) -> Result<Option<Transform2D>, LayoutError> {
        let mut chain = ancestor_chain(scene, node)?;
        chain.reverse();
        let mut transform = Transform2D::IDENTITY;
        for node in chain {
            let Some(geometry) = self.geometry(node) else {
                return Ok(None);
            };
            transform = transform.then(node_transform(scene, node, geometry)?);
        }
        Ok(Some(transform))
    }

    /// Removes geometry for destroyed or replaced scene nodes.
    pub fn retain_scene(&mut self, scene: &Scene) {
        self.geometry.retain(|node, _| scene.element(*node).is_ok());
    }
}

impl TransformWatcher {
    /// Creates a watcher between two nodes with an optional known common parent.
    pub fn new(a: NodeHandle, b: NodeHandle, common_parent: Option<NodeHandle>) -> Self {
        Self {
            a,
            b,
            common_parent,
            signature: None,
        }
    }

    /// Updates the watcher and reports a change after its initial observation.
    pub fn observe(
        &mut self,
        scene: &Scene,
        tracker: &TransformTracker,
    ) -> Result<bool, LayoutError> {
        let Some(signature) =
            transform_signature(scene, tracker, self.a, self.b, self.common_parent)?
        else {
            return Ok(false);
        };
        let changed = self.signature.is_some_and(|previous| previous != signature);
        self.signature = Some(signature);
        Ok(changed)
    }
}

fn transform_signature(
    scene: &Scene,
    tracker: &TransformTracker,
    a: NodeHandle,
    b: NodeHandle,
    common_parent: Option<NodeHandle>,
) -> Result<Option<u64>, LayoutError> {
    let a_chain = ancestor_chain(scene, a)?;
    let b_chain = ancestor_chain(scene, b)?;
    let common = if let Some(common) = common_parent {
        if !a_chain.contains(&common) || !b_chain.contains(&common) {
            return Err(LayoutError::InvalidCommonParent);
        }
        Some(common)
    } else {
        a_chain.iter().copied().find(|node| b_chain.contains(node))
    };
    let path = if let Some(common) = common {
        let mut path = a_chain
            .into_iter()
            .take_while(|node| *node != common)
            .collect::<Vec<_>>();
        path.push(common);
        path.extend(b_chain.into_iter().take_while(|node| *node != common));
        path
    } else {
        let mut path = a_chain;
        path.extend(b_chain);
        path
    };
    let mut hasher = DefaultHasher::new();
    for node in path {
        let Some(geometry) = tracker.geometry.get(&node) else {
            return Ok(None);
        };
        node.hash(&mut hasher);
        for value in [
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            scene.number(node, "rotation")?,
            scene.number(node, "scale")?,
            scene.number(node, "scale_x")?,
            scene.number(node, "scale_y")?,
            scene.number(node, "skew_x")?,
            scene.number(node, "skew_y")?,
            scene.number(node, "translate_x")?,
            scene.number(node, "translate_y")?,
            scene.number(node, "transform_origin_x")?,
            scene.number(node, "transform_origin_y")?,
        ] {
            value.to_bits().hash(&mut hasher);
        }
    }
    Ok(Some(hasher.finish()))
}

fn ancestor_chain(scene: &Scene, node: NodeHandle) -> Result<Vec<NodeHandle>, LayoutError> {
    let mut chain = Vec::new();
    let mut current = Some(node);
    while let Some(node) = current {
        chain.push(node);
        current = scene.parent(node)?;
    }
    Ok(chain)
}

pub(crate) fn inset_margin(
    scene: &Scene,
    node: NodeHandle,
    property: &'static str,
) -> Result<f64, LayoutError> {
    let margin = match scene.current(node, property)? {
        Value::Nil => scene.number(node, "margin")?,
        Value::Number(value) if value.is_finite() => *value,
        _ => return Err(LayoutError::InvalidInsetMargin(property)),
    };
    Ok(margin + scene.number(node, "extra_margin")?)
}

pub(crate) fn distributed_margin(available: f64, leading: f64, trailing: f64) -> f64 {
    let total = leading + trailing;
    let ratio = if total == 0.0 { 0.5 } else { leading / total };
    (available * ratio).round()
}

pub fn node_transform(
    scene: &Scene,
    node: NodeHandle,
    geometry: Geometry,
) -> Result<Transform2D, LayoutError> {
    let scale = scene.number(node, "scale")?;
    Ok(Transform2D::affine(
        (
            geometry.x + geometry.width * scene.number(node, "transform_origin_x")?,
            geometry.y + geometry.height * scene.number(node, "transform_origin_y")?,
        ),
        TransformParameters {
            translation: [
                scene.number(node, "translate_x")?,
                scene.number(node, "translate_y")?,
            ],
            scale: [
                scale * scene.number(node, "scale_x")?,
                scale * scene.number(node, "scale_y")?,
            ],
            rotation: scene.number(node, "rotation")?,
            skew: [scene.number(node, "skew_x")?, scene.number(node, "skew_y")?],
        },
    ))
}
