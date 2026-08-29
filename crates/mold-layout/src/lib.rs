//! Three-stage layout for mold scene nodes.

use std::collections::{BTreeMap, HashMap};
use std::error::Error as StdError;
use std::fmt;

use mold_scene::{Element, NodeHandle, Scene, SceneError, Value};

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
            Element::Item | Element::Rect => {
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
}
