//! Three-stage layout for mold scene nodes.

use std::collections::{BTreeMap, HashMap};
use std::error::Error as StdError;
use std::fmt;

use mold_scene::{Behavior, Element, NodeHandle, Scene, SceneError, Value};

/// Logical dimensions in surface coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
}

/// Resolved logical geometry for one node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Geometry {
    /// Horizontal offset from the parent.
    pub x: f64,
    /// Vertical offset from the parent.
    pub y: f64,
    /// Resolved width.
    pub width: f64,
    /// Resolved height.
    pub height: f64,
}

/// Text measurement supplied by the text subsystem.
pub trait TextMeasurer {
    /// Shapes text and returns its logical bounds.
    fn measure(
        &mut self,
        node: NodeHandle,
        text: &str,
        family: &str,
        size: f64,
        wrap_width: Option<f64>,
    ) -> Size;
}

/// Complete layout output keyed by stable node handles.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    geometry: HashMap<NodeHandle, Geometry>,
    implicit: HashMap<NodeHandle, Size>,
}

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
    /// Resolves the layout rooted at `root` into the supplied surface area.
    pub fn compute(
        scene: &Scene,
        root: NodeHandle,
        available: Size,
        text: &mut impl TextMeasurer,
    ) -> Result<Self, LayoutError> {
        let mut layout = Self::default();
        layout.measure_implicit(scene, root, text)?;
        layout.geometry.insert(
            root,
            Geometry {
                width: available.width,
                height: available.height,
                ..Geometry::default()
            },
        );
        layout.resolve_children(scene, root)?;
        Ok(layout)
    }

    /// Returns the resolved geometry for a node in the computed tree.
    pub fn geometry(&self, node: NodeHandle) -> Option<Geometry> {
        self.geometry.get(&node).copied()
    }

    /// Returns the bottom-up implicit size for a node.
    pub fn implicit_size(&self, node: NodeHandle) -> Option<Size> {
        self.implicit.get(&node).copied()
    }

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
        scene.reparent(transition.node, Some(transition.new_parent))?;
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

    /// Returns the topmost enabled MouseArea containing a surface-local point.
    pub fn hit_test(
        &self,
        scene: &Scene,
        x: f64,
        y: f64,
    ) -> Result<Option<NodeHandle>, LayoutError> {
        for root in scene.roots().into_iter().rev() {
            if let Some(node) = self.hit_node(scene, root, x, y)? {
                return Ok(Some(node));
            }
        }
        Ok(None)
    }

    /// Collects enabled MouseArea rectangles for the Wayland input region.
    pub fn input_geometry(&self, scene: &Scene) -> Result<Vec<Geometry>, LayoutError> {
        let mut rectangles = Vec::new();
        for root in scene.roots() {
            self.collect_input_geometry(scene, root, &mut rectangles)?;
        }
        Ok(rectangles)
    }

    fn collect_input_geometry(
        &self,
        scene: &Scene,
        node: NodeHandle,
        rectangles: &mut Vec<Geometry>,
    ) -> Result<(), LayoutError> {
        if !scene.bool_value(node, "visible")? || !scene.bool_value(node, "enabled")? {
            return Ok(());
        }
        if scene.element(node)? == Element::MouseArea
            && let Some(geometry) = self.geometry(node)
        {
            rectangles.push(geometry);
        }
        for child in scene.children(node)? {
            self.collect_input_geometry(scene, child, rectangles)?;
        }
        Ok(())
    }

    fn hit_node(
        &self,
        scene: &Scene,
        node: NodeHandle,
        x: f64,
        y: f64,
    ) -> Result<Option<NodeHandle>, LayoutError> {
        if !scene.bool_value(node, "visible")? || !scene.bool_value(node, "enabled")? {
            return Ok(None);
        }
        let Some(geometry) = self.geometry(node) else {
            return Ok(None);
        };
        if x < geometry.x
            || y < geometry.y
            || x >= geometry.x + geometry.width
            || y >= geometry.y + geometry.height
        {
            return Ok(None);
        }
        for child in scene.children(node)?.into_iter().rev() {
            if let Some(hit) = self.hit_node(scene, child, x, y)? {
                return Ok(Some(hit));
            }
        }
        Ok((scene.element(node)? == Element::MouseArea).then_some(node))
    }

    fn measure_implicit(
        &mut self,
        scene: &Scene,
        node: NodeHandle,
        text: &mut impl TextMeasurer,
    ) -> Result<Size, LayoutError> {
        let children = scene.children(node)?;
        let mut child_sizes = Vec::with_capacity(children.len());
        for child in &children {
            let implicit = self.measure_implicit(scene, *child, text)?;
            child_sizes.push(self.requested_size(scene, *child, implicit)?);
        }

        let size = match scene.element(node)? {
            Element::Text => text.measure(
                node,
                scene.string_value(node, "text")?,
                scene.string_value(node, "font_family")?,
                scene.number(node, "font_size")?,
                positive(scene.number(node, "width")?),
            ),
            Element::Row => Size {
                width: sum_with_spacing(&child_sizes, scene.number(node, "spacing")?, true),
                height: child_sizes
                    .iter()
                    .map(|size| size.height)
                    .fold(0.0, f64::max),
            },
            Element::Column => Size {
                width: child_sizes
                    .iter()
                    .map(|size| size.width)
                    .fold(0.0, f64::max),
                height: sum_with_spacing(&child_sizes, scene.number(node, "spacing")?, false),
            },
            Element::Item | Element::Rect | Element::MouseArea => {
                let mut bounds = Size::default();
                for (child, size) in children.iter().zip(child_sizes) {
                    bounds.width = bounds.width.max(scene.number(*child, "x")? + size.width);
                    bounds.height = bounds.height.max(scene.number(*child, "y")? + size.height);
                }
                bounds
            }
        };
        self.implicit.insert(node, size);
        Ok(size)
    }

    fn requested_size(
        &self,
        scene: &Scene,
        node: NodeHandle,
        implicit: Size,
    ) -> Result<Size, LayoutError> {
        Ok(Size {
            width: positive(scene.number(node, "width")?).unwrap_or(implicit.width),
            height: positive(scene.number(node, "height")?).unwrap_or(implicit.height),
        })
    }

    fn resolve_children(&mut self, scene: &Scene, parent: NodeHandle) -> Result<(), LayoutError> {
        let parent_geometry = self.geometry[&parent];
        let parent_element = scene.element(parent)?;
        let children = scene.children(parent)?;
        let spacing = match parent_element {
            Element::Row | Element::Column => scene.number(parent, "spacing")?,
            _ => 0.0,
        };
        let mut cursor = 0.0;

        for child in children {
            let implicit = self.implicit[&child];
            let size = self.requested_size(scene, child, implicit)?;
            let anchors = anchors(scene.current(child, "anchors")?)?;
            reject_axis_conflict(parent_element, &anchors)?;
            let mut geometry = Geometry {
                x: scene.number(child, "x")?,
                y: scene.number(child, "y")?,
                width: size.width,
                height: size.height,
            };
            apply_anchors(parent_geometry, &anchors, &mut geometry);
            match parent_element {
                Element::Row => {
                    geometry.x = cursor;
                    cursor += geometry.width + spacing;
                }
                Element::Column => {
                    geometry.y = cursor;
                    cursor += geometry.height + spacing;
                }
                _ => {}
            }
            geometry.x += parent_geometry.x;
            geometry.y += parent_geometry.y;
            geometry.x += scene.number(child, "transition_x")?;
            geometry.y += scene.number(child, "transition_y")?;
            self.geometry.insert(child, geometry);
            self.resolve_children(scene, child)?;
        }
        Ok(())
    }
}

/// A layout input or constraint failure.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutError {
    /// The scene graph rejected a read.
    Scene(String),
    /// The anchors property was not a string-keyed table.
    InvalidAnchors,
    /// Anchors and a positioner both control the same axis.
    AxisConflict { axis: &'static str },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(message) => write!(f, "scene layout error: {message}"),
            Self::InvalidAnchors => f.write_str("anchors must be a string-keyed map"),
            Self::AxisConflict { axis } => {
                write!(f, "anchors and positioner both control the {axis} axis")
            }
        }
    }
}

impl StdError for LayoutError {}

impl From<SceneError> for LayoutError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error.to_string())
    }
}

fn positive(value: f64) -> Option<f64> {
    (value > 0.0).then_some(value)
}

fn sum_with_spacing(children: &[Size], spacing: f64, horizontal: bool) -> f64 {
    let content = children
        .iter()
        .map(|size| if horizontal { size.width } else { size.height })
        .sum::<f64>();
    content + spacing * children.len().saturating_sub(1) as f64
}

fn anchors(value: &Value) -> Result<BTreeMap<String, Value>, LayoutError> {
    match value {
        Value::Map(map) => Ok(map.clone()),
        _ => Err(LayoutError::InvalidAnchors),
    }
}

fn reject_axis_conflict(
    parent: Element,
    anchors: &BTreeMap<String, Value>,
) -> Result<(), LayoutError> {
    let fill = flag(anchors, "fill");
    let center = flag(anchors, "center_in");
    if parent == Element::Row && (fill || center || flag(anchors, "left") || flag(anchors, "right"))
    {
        return Err(LayoutError::AxisConflict { axis: "horizontal" });
    }
    if parent == Element::Column
        && (fill || center || flag(anchors, "top") || flag(anchors, "bottom"))
    {
        return Err(LayoutError::AxisConflict { axis: "vertical" });
    }
    Ok(())
}

fn apply_anchors(parent: Geometry, anchors: &BTreeMap<String, Value>, geometry: &mut Geometry) {
    let margin = number(anchors, "margins").unwrap_or(0.0);
    let left_margin = number(anchors, "left_margin").unwrap_or(margin);
    let right_margin = number(anchors, "right_margin").unwrap_or(margin);
    let top_margin = number(anchors, "top_margin").unwrap_or(margin);
    let bottom_margin = number(anchors, "bottom_margin").unwrap_or(margin);
    if flag(anchors, "fill") {
        geometry.x = left_margin;
        geometry.y = top_margin;
        geometry.width = (parent.width - left_margin - right_margin).max(0.0);
        geometry.height = (parent.height - top_margin - bottom_margin).max(0.0);
        return;
    }
    if flag(anchors, "center_in") {
        geometry.x = (parent.width - geometry.width) / 2.0;
        geometry.y = (parent.height - geometry.height) / 2.0;
    }
    if flag(anchors, "left") {
        geometry.x = left_margin;
        if flag(anchors, "right") {
            geometry.width = (parent.width - left_margin - right_margin).max(0.0);
        }
    } else if flag(anchors, "right") {
        geometry.x = parent.width - geometry.width - right_margin;
    }
    if flag(anchors, "top") {
        geometry.y = top_margin;
        if flag(anchors, "bottom") {
            geometry.height = (parent.height - top_margin - bottom_margin).max(0.0);
        }
    } else if flag(anchors, "bottom") {
        geometry.y = parent.height - geometry.height - bottom_margin;
    }
}

fn flag(map: &BTreeMap<String, Value>, key: &str) -> bool {
    matches!(map.get(key), Some(Value::Bool(true)))
}

fn number(map: &BTreeMap<String, Value>, key: &str) -> Option<f64> {
    match map.get(key) {
        Some(Value::Number(value)) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedText;

    impl TextMeasurer for FixedText {
        fn measure(
            &mut self,
            _node: NodeHandle,
            text: &str,
            _family: &str,
            size: f64,
            _wrap_width: Option<f64>,
        ) -> Size {
            Size {
                width: text.len() as f64 * size / 2.0,
                height: size,
            }
        }
    }

    #[test]
    fn implicit_sizes_resolve_bottom_up_before_rows_distribute() {
        let mut scene = Scene::new();
        let row = scene.create(Element::Row);
        scene.assign(row, "spacing", 5.0).unwrap();
        let first = scene.create(Element::Text);
        scene.assign(first, "text", "aa").unwrap();
        scene.assign(first, "font_size", 10.0).unwrap();
        let second = scene.create(Element::Rect);
        scene.assign(second, "width", 20.0).unwrap();
        scene.assign(second, "height", 8.0).unwrap();
        scene.reparent(first, Some(row)).unwrap();
        scene.reparent(second, Some(row)).unwrap();

        let layout = Layout::compute(
            &scene,
            row,
            Size {
                width: 100.0,
                height: 20.0,
            },
            &mut FixedText,
        )
        .unwrap();

        assert_eq!(layout.implicit_size(row).unwrap().width, 35.0);
        assert_eq!(layout.geometry(first).unwrap().x, 0.0);
        assert_eq!(layout.geometry(second).unwrap().x, 15.0);
    }

    #[test]
    fn fill_anchors_respect_margins() {
        let mut scene = Scene::new();
        let parent = scene.create(Element::Item);
        let child = scene.create(Element::Rect);
        let anchors = BTreeMap::from([
            ("fill".to_owned(), Value::Bool(true)),
            ("margins".to_owned(), Value::Number(4.0)),
        ]);
        scene.assign(child, "anchors", Value::Map(anchors)).unwrap();
        scene.reparent(child, Some(parent)).unwrap();

        let layout = Layout::compute(
            &scene,
            parent,
            Size {
                width: 80.0,
                height: 40.0,
            },
            &mut FixedText,
        )
        .unwrap();

        assert_eq!(
            layout.geometry(child).unwrap(),
            Geometry {
                x: 4.0,
                y: 4.0,
                width: 72.0,
                height: 32.0,
            }
        );
    }

    #[test]
    fn anchors_cannot_compete_with_a_positioner_axis() {
        let mut scene = Scene::new();
        let row = scene.create(Element::Row);
        let child = scene.create(Element::Item);
        scene
            .assign(
                child,
                "anchors",
                Value::Map(BTreeMap::from([("left".to_owned(), Value::Bool(true))])),
            )
            .unwrap();
        scene.reparent(child, Some(row)).unwrap();

        let error = Layout::compute(
            &scene,
            row,
            Size {
                width: 100.0,
                height: 20.0,
            },
            &mut FixedText,
        )
        .unwrap_err();

        assert_eq!(error, LayoutError::AxisConflict { axis: "horizontal" });
    }

    #[test]
    fn hit_test_uses_absolute_geometry_and_paint_order() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Item);
        let parent = scene.create(Element::Item);
        scene.assign(parent, "x", 10.0).unwrap();
        scene.assign(parent, "y", 5.0).unwrap();
        scene.assign(parent, "width", 40.0).unwrap();
        scene.assign(parent, "height", 20.0).unwrap();
        scene.reparent(parent, Some(root)).unwrap();
        let first = scene.create(Element::MouseArea);
        let second = scene.create(Element::MouseArea);
        for area in [first, second] {
            scene.assign(area, "x", 3.0).unwrap();
            scene.assign(area, "y", 2.0).unwrap();
            scene.assign(area, "width", 20.0).unwrap();
            scene.assign(area, "height", 10.0).unwrap();
            scene.reparent(area, Some(parent)).unwrap();
        }
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 100.0,
                height: 40.0,
            },
            &mut FixedText,
        )
        .unwrap();

        assert_eq!(layout.geometry(second).unwrap().x, 13.0);
        assert_eq!(layout.hit_test(&scene, 15.0, 9.0).unwrap(), Some(second));
        scene.assign(second, "enabled", false).unwrap();
        assert_eq!(layout.hit_test(&scene, 15.0, 9.0).unwrap(), Some(first));
        assert_eq!(layout.hit_test(&scene, 2.0, 2.0).unwrap(), None);
        assert_eq!(
            layout.input_geometry(&scene).unwrap(),
            vec![layout.geometry(first).unwrap()]
        );
    }

    #[test]
    fn reparent_transition_preserves_position_then_flies_to_target() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Item);
        let left = scene.create(Element::Item);
        let right = scene.create(Element::Item);
        let tile = scene.create(Element::Rect);
        scene.assign(left, "x", 10.0).unwrap();
        scene.assign(left, "width", 100.0).unwrap();
        scene.assign(left, "height", 100.0).unwrap();
        scene.assign(right, "x", 200.0).unwrap();
        scene.assign(right, "width", 100.0).unwrap();
        scene.assign(right, "height", 100.0).unwrap();
        scene.assign(tile, "x", 5.0).unwrap();
        scene.assign(tile, "width", 20.0).unwrap();
        scene.assign(tile, "height", 20.0).unwrap();
        scene.reparent(left, Some(root)).unwrap();
        scene.reparent(right, Some(root)).unwrap();
        scene.reparent(tile, Some(left)).unwrap();
        let available = Size {
            width: 400.0,
            height: 200.0,
        };
        let behavior = Behavior {
            duration: std::time::Duration::from_millis(200),
            easing: mold_scene::Easing::Linear,
        };

        let initial = Layout::transition_reparent(
            &mut scene,
            &mut FixedText,
            ReparentTransition {
                root,
                node: tile,
                new_parent: right,
                anchors: None,
                available,
                behavior,
            },
        )
        .unwrap();

        assert_eq!(scene.parent(tile).unwrap(), Some(right));
        assert_eq!(initial.geometry(tile).unwrap().x, 15.0);
        scene
            .tick_animations(std::time::Duration::from_millis(100))
            .unwrap();
        let halfway = Layout::compute(&scene, root, available, &mut FixedText).unwrap();
        assert_eq!(halfway.geometry(tile).unwrap().x, 110.0);
        scene
            .tick_animations(std::time::Duration::from_millis(100))
            .unwrap();
        let finished = Layout::compute(&scene, root, available, &mut FixedText).unwrap();
        assert_eq!(finished.geometry(tile).unwrap().x, 205.0);
    }
}
