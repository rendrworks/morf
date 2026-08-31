use mold_scene::{Element, NodeHandle, Scene};

use crate::geometry::{Geometry, Transform2D};
use crate::helpers::LayoutError;
use crate::layout::Layout;
use crate::transform::node_transform;

/// One hit-tested MouseArea together with the tested point inside that node.
///
/// The two coordinate spaces are deliberately distinct. A hit test is queried
/// in *surface* space — the coordinates the compositor delivers, shared by
/// every node on the surface — while `local_x`/`local_y` are the same point
/// expressed inside the node that was hit, so a handler can divide by its own
/// width without knowing where any ancestor placed it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The topmost enabled MouseArea containing the point.
    pub node: NodeHandle,
    /// Point x inside `node`: `0.0` at its left edge, its width at the right,
    /// with every ancestor offset and transform already removed.
    pub local_x: f64,
    /// Point y inside `node`: `0.0` at its top edge, its height at the bottom.
    pub local_y: f64,
}

impl Layout {
    /// Returns the topmost enabled MouseArea containing a surface-local point.
    pub fn hit_test(&self, scene: &Scene, x: f64, y: f64) -> Result<Option<Hit>, LayoutError> {
        for root in scene.roots().into_iter().rev() {
            if let Some(hit) = self.hit_node(scene, root, Transform2D::IDENTITY, x, y)? {
                return Ok(Some(hit));
            }
        }
        Ok(None)
    }

    /// Converts a surface-local point into one node's own coordinates.
    ///
    /// Unlike [`Layout::hit_test`] the point need not land on the node, so a
    /// drag that has pulled off its handle still reports where the pointer is
    /// relative to that handle — negative to the left of it, past its width to
    /// the right. A node with no resolved geometry or a singular transform is
    /// unreachable by a pointer; the surface point is returned unchanged for
    /// it rather than failing the event.
    pub fn local_point(&self, scene: &Scene, node: NodeHandle, x: f64, y: f64) -> (f64, f64) {
        let Some(geometry) = self.geometry(node) else {
            return (x, y);
        };
        let Ok(transform) = self.chain_transform(scene, node) else {
            return (x, y);
        };
        transform
            .inverse_point(x, y)
            .map_or((x, y), |(local_x, local_y)| {
                (local_x - geometry.x, local_y - geometry.y)
            })
    }

    /// Collects enabled MouseArea rectangles for the Wayland input region.
    pub fn input_geometry(&self, scene: &Scene) -> Result<Vec<Geometry>, LayoutError> {
        let mut rectangles = Vec::new();
        for root in scene.roots() {
            self.collect_input_geometry(scene, root, Transform2D::IDENTITY, &mut rectangles)?;
        }
        Ok(rectangles)
    }

    /// Accumulates the transform chain from the scene root down to one node.
    fn chain_transform(&self, scene: &Scene, node: NodeHandle) -> Result<Transform2D, LayoutError> {
        let mut chain = vec![node];
        let mut current = node;
        while let Some(parent) = scene.parent(current)? {
            chain.push(parent);
            current = parent;
        }
        let mut transform = Transform2D::IDENTITY;
        for node in chain.into_iter().rev() {
            let Some(geometry) = self.geometry(node) else {
                return Err(LayoutError::Scene(
                    "node has no resolved geometry".to_owned(),
                ));
            };
            transform = transform.then(node_transform(scene, node, geometry)?);
        }
        Ok(transform)
    }

    fn collect_input_geometry(
        &self,
        scene: &Scene,
        node: NodeHandle,
        inherited: Transform2D,
        rectangles: &mut Vec<Geometry>,
    ) -> Result<(), LayoutError> {
        if !scene.bool_value(node, "visible")? || !scene.bool_value(node, "enabled")? {
            return Ok(());
        }
        let Some(geometry) = self.geometry(node) else {
            return Ok(());
        };
        let transform = inherited.then(node_transform(scene, node, geometry)?);
        if scene.element(node)? == Element::MouseArea
            && let Some(geometry) = self.geometry(node)
        {
            rectangles.push(transform.bounds(geometry));
        }
        for &child in scene.children(node)? {
            self.collect_input_geometry(scene, child, transform, rectangles)?;
        }
        Ok(())
    }

    fn hit_node(
        &self,
        scene: &Scene,
        node: NodeHandle,
        inherited: Transform2D,
        x: f64,
        y: f64,
    ) -> Result<Option<Hit>, LayoutError> {
        if !scene.bool_value(node, "visible")? || !scene.bool_value(node, "enabled")? {
            return Ok(None);
        }
        let Some(geometry) = self.geometry(node) else {
            return Ok(None);
        };
        let transform = inherited.then(node_transform(scene, node, geometry)?);
        let Some((local_x, local_y)) = transform.inverse_point(x, y) else {
            return Ok(None);
        };
        let inside = local_x >= geometry.x
            && local_y >= geometry.y
            && local_x < geometry.x + geometry.width
            && local_y < geometry.y + geometry.height;
        if !inside && scene.bool_value(node, "clip")? {
            return Ok(None);
        }
        for &child in scene.children(node)?.iter().rev() {
            if let Some(hit) = self.hit_node(scene, child, transform, x, y)? {
                return Ok(Some(hit));
            }
        }
        // The inverse point is measured in the space the node's own geometry
        // is resolved in, which is absolute; subtracting the node's origin is
        // what makes it node-local.
        Ok(
            (inside && scene.element(node)? == Element::MouseArea).then_some(Hit {
                node,
                local_x: local_x - geometry.x,
                local_y: local_y - geometry.y,
            }),
        )
    }
}
