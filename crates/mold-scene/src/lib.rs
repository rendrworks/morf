//! Scene graph, typed properties, and animation targets for mold.

use std::collections::{BTreeMap, HashMap};
use std::error::Error as StdError;
use std::fmt;

use mold_reactive::{Graph, GraphError, SignalId};
use slotmap::{SlotMap, new_key_type};

new_key_type! {
    struct NodeId;
}

/// A generational scene node handle safe to retain outside the arena.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NodeHandle(NodeId);

/// Element kinds implemented by the first scene milestone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Element {
    /// Non-painting container.
    Item,
    /// Rounded rectangle primitive.
    Rect,
    /// Shaped text primitive.
    Text,
    /// Sequential horizontal positioner.
    Row,
    /// Sequential vertical positioner.
    Column,
}

impl Element {
    fn name(self) -> &'static str {
        match self {
            Self::Item => "Item",
            Self::Rect => "Rect",
            Self::Text => "Text",
            Self::Row => "Row",
            Self::Column => "Column",
        }
    }
}

/// Linear RGBA colour with components in the inclusive zero-to-one range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Red channel.
    pub red: f32,
    /// Green channel.
    pub green: f32,
    /// Blue channel.
    pub blue: f32,
    /// Alpha channel.
    pub alpha: f32,
}

impl Color {
    /// Creates a colour from eight-bit channels.
    pub const fn rgba8(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red: red as f32 / 255.0,
            green: green as f32 / 255.0,
            blue: blue as f32 / 255.0,
            alpha: alpha as f32 / 255.0,
        }
    }

    fn parse(input: &str) -> Option<Self> {
        match input {
            "transparent" => return Some(Self::rgba8(0, 0, 0, 0)),
            "black" => return Some(Self::rgba8(0, 0, 0, 255)),
            "white" => return Some(Self::rgba8(255, 255, 255, 255)),
            "red" => return Some(Self::rgba8(255, 0, 0, 255)),
            "green" => return Some(Self::rgba8(0, 128, 0, 255)),
            "blue" => return Some(Self::rgba8(0, 0, 255, 255)),
            _ => {}
        }
        let hex = input.strip_prefix('#')?;
        let expand = |byte: u8| (byte << 4) | byte;
        let nibble = |at: usize| u8::from_str_radix(&hex[at..at + 1], 16).ok();
        let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        let (red, green, blue, alpha) = match hex.len() {
            3 => (
                expand(nibble(0)?),
                expand(nibble(1)?),
                expand(nibble(2)?),
                255,
            ),
            4 => (
                expand(nibble(0)?),
                expand(nibble(1)?),
                expand(nibble(2)?),
                expand(nibble(3)?),
            ),
            6 => (byte(0)?, byte(2)?, byte(4)?, 255),
            8 => (byte(0)?, byte(2)?, byte(4)?, byte(6)?),
            _ => return None,
        };
        Some(Self::rgba8(red, green, blue, alpha))
    }
}

/// Values stored in reactive element properties.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// No value.
    Nil,
    /// Boolean value.
    Bool(bool),
    /// Floating-point number.
    Number(f64),
    /// UTF-8 string.
    String(String),
    /// Normalized RGBA colour.
    Color(Color),
    /// Ordered sequence used by declarative data.
    List(Vec<Value>),
    /// String-keyed declarative data such as anchors.
    Map(BTreeMap<String, Value>),
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Number(value as f64)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

/// Scene arena and its property signal graph.
pub struct Scene {
    nodes: SlotMap<NodeId, Node>,
    properties: Graph<Value>,
}

struct Node {
    element: Element,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    properties: HashMap<&'static str, PropertySlot>,
}

#[derive(Clone, Copy)]
struct PropertySlot {
    current: SignalId,
    target: SignalId,
    kind: PropertyType,
}

#[derive(Clone, Copy)]
enum PropertyType {
    Any,
    Bool,
    Number,
    String,
    Color,
}

#[derive(Clone)]
struct PropertySpec {
    name: &'static str,
    kind: PropertyType,
    default: Value,
}

/// A scene graph operation failure.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneError {
    /// A node handle no longer refers to a live node.
    StaleNode,
    /// A node cannot become a child of itself or one of its descendants.
    ParentCycle,
    /// The named property is absent from the element schema.
    UnknownProperty {
        /// Element type receiving the assignment.
        element: &'static str,
        /// Rejected property name.
        property: String,
    },
    /// A property value cannot be converted to its declared type.
    InvalidPropertyType {
        /// Element type receiving the assignment.
        element: &'static str,
        /// Property whose coercion failed.
        property: String,
        /// Required property type.
        expected: &'static str,
    },
    /// The property signal graph rejected an operation.
    Reactive(String),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleNode => f.write_str("stale scene node handle"),
            Self::ParentCycle => f.write_str("a node cannot parent itself or its ancestor"),
            Self::UnknownProperty { element, property } => {
                write!(f, "unknown {element} property `{property}`")
            }
            Self::InvalidPropertyType {
                element,
                property,
                expected,
            } => write!(
                f,
                "invalid {element} property `{property}`: expected {expected}"
            ),
            Self::Reactive(message) => write!(f, "reactive property error: {message}"),
        }
    }
}

impl StdError for SceneError {}

impl From<GraphError> for SceneError {
    fn from(error: GraphError) -> Self {
        Self::Reactive(error.to_string())
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    /// Creates an empty scene arena.
    pub fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
            properties: Graph::default(),
        }
    }

    /// Allocates an element with every schema property initialized.
    pub fn create(&mut self, element: Element) -> NodeHandle {
        let node = self.nodes.insert_with_key(|id| {
            let properties = schema(element)
                .into_iter()
                .map(|spec| {
                    let prefix = format!("{}[{:?}].{}", element.name(), id, spec.name);
                    let current = self
                        .properties
                        .signal(format!("{prefix}.current"), spec.default.clone());
                    let target = self
                        .properties
                        .signal(format!("{prefix}.target"), spec.default);
                    (
                        spec.name,
                        PropertySlot {
                            current,
                            target,
                            kind: spec.kind,
                        },
                    )
                })
                .collect();
            Node {
                element,
                parent: None,
                children: Vec::new(),
                properties,
            }
        });
        NodeHandle(node)
    }

    /// Returns whether a handle still refers to a live node generation.
    pub fn contains(&self, node: NodeHandle) -> bool {
        self.nodes.contains_key(node.0)
    }

    /// Returns all live nodes without a parent in arena order.
    pub fn roots(&self) -> Vec<NodeHandle> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.parent.is_none())
            .map(|(id, _)| NodeHandle(id))
            .collect()
    }

    /// Returns the element kind for a live node.
    pub fn element(&self, node: NodeHandle) -> Result<Element, SceneError> {
        Ok(self.nodes[self.live(node)?].element)
    }

    /// Checks whether a live element schema declares a property.
    pub fn has_property(&self, node: NodeHandle, property: &str) -> Result<bool, SceneError> {
        Ok(self.nodes[self.live(node)?]
            .properties
            .contains_key(property))
    }

    /// Appends a node to a new parent while preserving the child's identity.
    pub fn reparent(
        &mut self,
        child: NodeHandle,
        parent: Option<NodeHandle>,
    ) -> Result<(), SceneError> {
        let child_id = self.live(child)?;
        let parent_id = parent.map(|handle| self.live(handle)).transpose()?;
        if parent_id == Some(child_id) {
            return Err(SceneError::ParentCycle);
        }
        let mut ancestor = parent_id;
        while let Some(node) = ancestor {
            if node == child_id {
                return Err(SceneError::ParentCycle);
            }
            ancestor = self.nodes[node].parent;
        }

        if let Some(old_parent) = self.nodes[child_id].parent {
            self.nodes[old_parent]
                .children
                .retain(|node| *node != child_id);
        }
        self.nodes[child_id].parent = parent_id;
        if let Some(parent) = parent_id {
            self.nodes[parent].children.push(child_id);
        }
        Ok(())
    }

    /// Returns the current parent handle.
    pub fn parent(&self, node: NodeHandle) -> Result<Option<NodeHandle>, SceneError> {
        Ok(self.nodes[self.live(node)?].parent.map(NodeHandle))
    }

    /// Returns child handles in paint order.
    pub fn children(&self, node: NodeHandle) -> Result<Vec<NodeHandle>, SceneError> {
        Ok(self.nodes[self.live(node)?]
            .children
            .iter()
            .copied()
            .map(NodeHandle)
            .collect())
    }

    /// Removes a node and all descendants, invalidating their handles.
    pub fn remove(&mut self, node: NodeHandle) -> Result<(), SceneError> {
        let id = self.live(node)?;
        if let Some(parent) = self.nodes[id].parent {
            self.nodes[parent].children.retain(|child| *child != id);
        }
        let mut pending = vec![id];
        while let Some(current) = pending.pop() {
            pending.extend(self.nodes[current].children.iter().copied());
            self.nodes.remove(current);
        }
        Ok(())
    }

    /// Assigns and coerces a plain value to both target and rendered property levels.
    pub fn assign(
        &mut self,
        node: NodeHandle,
        property: &str,
        value: impl Into<Value>,
    ) -> Result<(), SceneError> {
        let id = self.live(node)?;
        let element = self.nodes[id].element;
        let slot = self.nodes[id]
            .properties
            .get(property)
            .copied()
            .ok_or_else(|| SceneError::UnknownProperty {
                element: element.name(),
                property: property.to_owned(),
            })?;
        let value = coerce(element, property, slot.kind, value.into())?;
        self.properties.batch(|graph| {
            graph.write(slot.target, value.clone())?;
            graph.write(slot.current, value)?;
            Ok(())
        })?;
        Ok(())
    }

    /// Reads the value currently used by layout or paint.
    pub fn current(&self, node: NodeHandle, property: &str) -> Result<&Value, SceneError> {
        let slot = self.property(node, property)?;
        Ok(self.properties.read(slot.current)?)
    }

    /// Reads the settled value most recently produced by a binding or assignment.
    pub fn target(&self, node: NodeHandle, property: &str) -> Result<&Value, SceneError> {
        let slot = self.property(node, property)?;
        Ok(self.properties.read(slot.target)?)
    }

    /// Reads a numeric current property.
    pub fn number(&self, node: NodeHandle, property: &str) -> Result<f64, SceneError> {
        match self.current(node, property)? {
            Value::Number(value) => Ok(*value),
            _ => Err(SceneError::Reactive(format!(
                "property `{property}` is not numeric"
            ))),
        }
    }

    /// Reads a string current property.
    pub fn string_value(&self, node: NodeHandle, property: &str) -> Result<&str, SceneError> {
        match self.current(node, property)? {
            Value::String(value) => Ok(value),
            _ => Err(SceneError::Reactive(format!(
                "property `{property}` is not a string"
            ))),
        }
    }

    fn live(&self, node: NodeHandle) -> Result<NodeId, SceneError> {
        self.nodes
            .contains_key(node.0)
            .then_some(node.0)
            .ok_or(SceneError::StaleNode)
    }

    fn property(&self, node: NodeHandle, property: &str) -> Result<PropertySlot, SceneError> {
        let id = self.live(node)?;
        let element = self.nodes[id].element;
        self.nodes[id]
            .properties
            .get(property)
            .copied()
            .ok_or_else(|| SceneError::UnknownProperty {
                element: element.name(),
                property: property.to_owned(),
            })
    }
}

fn schema(element: Element) -> Vec<PropertySpec> {
    let mut properties = vec![
        number("x", 0.0),
        number("y", 0.0),
        number("width", 0.0),
        number("height", 0.0),
        number("implicit_width", 0.0),
        number("implicit_height", 0.0),
        any("anchors", Value::Map(BTreeMap::new())),
        boolean("visible", true),
        number("opacity", 1.0),
        number("z", 0.0),
        boolean("clip", false),
        number("rotation", 0.0),
        number("scale", 1.0),
        boolean("enabled", true),
    ];
    match element {
        Element::Item => {}
        Element::Rect => {
            properties.extend([
                color("color", Color::rgba8(255, 255, 255, 255)),
                number("radius", 0.0),
                number("border_width", 0.0),
                color("border_color", Color::rgba8(0, 0, 0, 0)),
            ]);
        }
        Element::Text => {
            properties.extend([
                string("text", ""),
                color("color", Color::rgba8(0, 0, 0, 255)),
                number("font_size", 16.0),
                string("font_family", "sans-serif"),
                boolean("wrap", false),
            ]);
        }
        Element::Row | Element::Column => {
            properties.push(number("spacing", 0.0));
        }
    }
    properties
}

fn any(name: &'static str, default: Value) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::Any,
        default,
    }
}

fn boolean(name: &'static str, default: bool) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::Bool,
        default: Value::Bool(default),
    }
}

fn number(name: &'static str, default: f64) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::Number,
        default: Value::Number(default),
    }
}

fn string(name: &'static str, default: &str) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::String,
        default: Value::String(default.to_owned()),
    }
}

fn color(name: &'static str, default: Color) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::Color,
        default: Value::Color(default),
    }
}

fn coerce(
    element: Element,
    property: &str,
    kind: PropertyType,
    value: Value,
) -> Result<Value, SceneError> {
    let converted = match (kind, value) {
        (PropertyType::Any, value) => Some(value),
        (PropertyType::Bool, Value::Bool(value)) => Some(Value::Bool(value)),
        (PropertyType::Number, Value::Number(value)) if value.is_finite() => {
            Some(Value::Number(value))
        }
        (PropertyType::String, Value::String(value)) => Some(Value::String(value)),
        (PropertyType::Color, Value::Color(value)) => Some(Value::Color(value)),
        (PropertyType::Color, Value::String(value)) => Color::parse(&value).map(Value::Color),
        _ => None,
    };
    converted.ok_or_else(|| SceneError::InvalidPropertyType {
        element: element.name(),
        property: property.to_owned(),
        expected: match kind {
            PropertyType::Any => "value",
            PropertyType::Bool => "boolean",
            PropertyType::Number => "finite number",
            PropertyType::String => "string",
            PropertyType::Color => "color",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reparenting_preserves_identity_and_order() {
        let mut scene = Scene::new();
        let first_parent = scene.create(Element::Item);
        let second_parent = scene.create(Element::Item);
        let child = scene.create(Element::Rect);
        scene.reparent(child, Some(first_parent)).unwrap();
        scene.reparent(child, Some(second_parent)).unwrap();

        assert!(scene.children(first_parent).unwrap().is_empty());
        assert_eq!(scene.children(second_parent).unwrap(), vec![child]);
        assert_eq!(scene.parent(child).unwrap(), Some(second_parent));
    }

    #[test]
    fn reparenting_rejects_descendant_cycles() {
        let mut scene = Scene::new();
        let parent = scene.create(Element::Item);
        let child = scene.create(Element::Item);
        scene.reparent(child, Some(parent)).unwrap();

        assert_eq!(
            scene.reparent(parent, Some(child)),
            Err(SceneError::ParentCycle)
        );
    }

    #[test]
    fn removed_handles_are_detectably_stale() {
        let mut scene = Scene::new();
        let parent = scene.create(Element::Item);
        let child = scene.create(Element::Text);
        scene.reparent(child, Some(parent)).unwrap();
        scene.remove(parent).unwrap();

        assert!(!scene.contains(parent));
        assert!(!scene.contains(child));
        assert_eq!(scene.parent(child), Err(SceneError::StaleNode));
    }

    #[test]
    fn properties_coerce_colors_and_update_both_levels() {
        let mut scene = Scene::new();
        let rect = scene.create(Element::Rect);
        scene.assign(rect, "color", "#7c3aed").unwrap();

        let expected = Value::Color(Color::rgba8(0x7c, 0x3a, 0xed, 0xff));
        assert_eq!(scene.current(rect, "color").unwrap(), &expected);
        assert_eq!(scene.target(rect, "color").unwrap(), &expected);
    }

    #[test]
    fn property_errors_name_the_element_and_property() {
        let mut scene = Scene::new();
        let text = scene.create(Element::Text);

        let unknown = scene.assign(text, "radius", 4.0).unwrap_err();
        assert_eq!(unknown.to_string(), "unknown Text property `radius`");
        let wrong = scene.assign(text, "font_size", "large").unwrap_err();
        assert!(wrong.to_string().contains("Text property `font_size`"));
    }
}
