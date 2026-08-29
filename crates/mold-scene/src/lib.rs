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
    /// Pointer and focus event target with no visual output.
    MouseArea,
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
            Self::MouseArea => "MouseArea",
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
    behaviors: HashMap<PropertyKey, Behavior>,
    animations: HashMap<PropertyKey, Animation>,
    physics: HashMap<PropertyKey, PhysicsAnimation>,
    physics_specs: HashMap<PropertyKey, Physics>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PropertyKey {
    node: NodeId,
    property: &'static str,
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

/// Frame-pipeline work invalidated by an animated property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertyClass {
    /// Node transform or compositor uniform only.
    Transform,
    /// Geometry requiring a layout pass.
    Layout,
    /// Draw-list data requiring repaint only.
    Paint,
}

/// Easing applied to a property behavior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    /// Constant interpolation rate.
    Linear,
    /// Cubic acceleration.
    InCubic,
    /// Cubic deceleration.
    OutCubic,
    /// Cubic acceleration followed by deceleration.
    InOutCubic,
    /// CSS-style cubic Bezier timing curve.
    CubicBezier {
        /// First control point x coordinate.
        x1: f64,
        /// First control point y coordinate.
        y1: f64,
        /// Second control point x coordinate.
        x2: f64,
        /// Second control point y coordinate.
        y2: f64,
    },
}

impl Easing {
    fn sample(self, progress: f64) -> f64 {
        let progress = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => progress,
            Self::InCubic => progress.powi(3),
            Self::OutCubic => 1.0 - (1.0 - progress).powi(3),
            Self::InOutCubic if progress < 0.5 => 4.0 * progress.powi(3),
            Self::InOutCubic => 1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0,
            Self::CubicBezier { x1, y1, x2, y2 } => cubic_bezier(progress, x1, y1, x2, y2),
        }
    }
}

/// Write interceptor installed on an animatable property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Behavior {
    /// Time from current value to the new target.
    pub duration: Duration,
    /// Timing curve for uninterrupted motion.
    pub easing: Easing,
}

/// Physics-driven motion installed on a numeric property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Physics {
    /// Damped spring that retains velocity when its target changes.
    Spring {
        /// Moving mass.
        mass: f64,
        /// Velocity damping coefficient.
        damping: f64,
        /// Restoring-force coefficient.
        stiffness: f64,
        /// Position and velocity threshold used to settle.
        epsilon: f64,
    },
    /// Constant-speed pursuit of the latest target.
    Smoothed {
        /// Maximum property units travelled per second.
        velocity: f64,
    },
}

/// One property changed by an animation tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimatedChange {
    /// Node whose current property value changed.
    pub node: NodeHandle,
    /// Property name.
    pub property: &'static str,
    /// Frame-pipeline work required by the property.
    pub class: PropertyClass,
}

/// Result of a pure-Rust animation clock tick.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnimationFrame {
    /// Properties whose current level advanced.
    pub changes: Vec<AnimatedChange>,
    /// Whether another compositor frame callback is required.
    pub active: bool,
}

#[derive(Clone, Debug)]
struct Animation {
    from: Value,
    to: Value,
    initial_velocity: Velocity,
    preserve_velocity: bool,
    elapsed: Duration,
    behavior: Behavior,
}

#[derive(Clone, Copy, Debug)]
struct PhysicsAnimation {
    target: f64,
    velocity: f64,
    spec: Physics,
}

#[derive(Clone, Copy, Debug)]
enum Velocity {
    Number(f64),
    Color([f64; 4]),
}

impl Velocity {
    fn is_moving(self) -> bool {
        match self {
            Self::Number(value) => value.abs() > 1e-6,
            Self::Color(values) => values.into_iter().any(|value| value.abs() > 1e-6),
        }
    }
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
            behaviors: HashMap::new(),
            animations: HashMap::new(),
            physics: HashMap::new(),
            physics_specs: HashMap::new(),
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
            self.behaviors.retain(|key, _| key.node != current);
            self.animations.retain(|key, _| key.node != current);
            self.physics.retain(|key, _| key.node != current);
            self.physics_specs.retain(|key, _| key.node != current);
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
        let (property_name, slot) = self.nodes[id]
            .properties
            .get_key_value(property)
            .map(|(name, slot)| (*name, *slot))
            .ok_or_else(|| SceneError::UnknownProperty {
                element: element.name(),
                property: property.to_owned(),
            })?;
        let value = coerce(element, property, slot.kind, value.into())?;
        let key = PropertyKey {
            node: id,
            property: property_name,
        };
        if self.properties.read(slot.target)? == &value {
            return Ok(());
        }
        if let Some(spec) = self.physics_specs.get(&key).copied()
            && let Value::Number(target) = value
            && matches!(self.properties.read(slot.current)?, Value::Number(_))
        {
            let velocity = self.physics.get(&key).map_or(0.0, |motion| motion.velocity);
            self.animations.remove(&key);
            self.properties.write(slot.target, Value::Number(target))?;
            self.physics.insert(
                key,
                PhysicsAnimation {
                    target,
                    velocity,
                    spec,
                },
            );
        } else if let Some(behavior) = self.behaviors.get(&key).copied()
            && behavior.duration > Duration::ZERO
            && interpolatable(self.properties.read(slot.current)?, &value)
        {
            let from = self.properties.read(slot.current)?.clone();
            let initial_velocity = self
                .animations
                .get(&key)
                .map(Animation::velocity)
                .unwrap_or_else(|| zero_velocity(&from));
            self.properties.write(slot.target, value.clone())?;
            self.animations.insert(
                key,
                Animation {
                    from,
                    to: value,
                    initial_velocity,
                    preserve_velocity: initial_velocity.is_moving(),
                    elapsed: Duration::ZERO,
                    behavior,
                },
            );
        } else {
            self.animations.remove(&key);
            self.physics.remove(&key);
            self.properties.batch(|graph| {
                graph.write(slot.target, value.clone())?;
                graph.write(slot.current, value)?;
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Installs or removes a write-intercepting behavior on a property.
    pub fn set_behavior(
        &mut self,
        node: NodeHandle,
        property: &str,
        behavior: Option<Behavior>,
    ) -> Result<(), SceneError> {
        let id = self.live(node)?;
        let (name, _) = self.nodes[id]
            .properties
            .get_key_value(property)
            .ok_or_else(|| SceneError::UnknownProperty {
                element: self.nodes[id].element.name(),
                property: property.to_owned(),
            })?;
        let key = PropertyKey {
            node: id,
            property: name,
        };
        if let Some(behavior) = behavior {
            self.behaviors.insert(key, behavior);
            self.physics_specs.remove(&key);
            self.physics.remove(&key);
        } else {
            self.behaviors.remove(&key);
            self.animations.remove(&key);
        }
        Ok(())
    }

    /// Starts a finite animation from an explicit current value.
    pub fn animate_from(
        &mut self,
        node: NodeHandle,
        property: &str,
        from: impl Into<Value>,
        to: impl Into<Value>,
        behavior: Behavior,
    ) -> Result<(), SceneError> {
        let id = self.live(node)?;
        let element = self.nodes[id].element;
        let (name, slot) = self.nodes[id]
            .properties
            .get_key_value(property)
            .map(|(name, slot)| (*name, *slot))
            .ok_or_else(|| SceneError::UnknownProperty {
                element: element.name(),
                property: property.to_owned(),
            })?;
        let from = coerce(element, property, slot.kind, from.into())?;
        let to = coerce(element, property, slot.kind, to.into())?;
        if !interpolatable(&from, &to) {
            return Err(SceneError::InvalidPropertyType {
                element: element.name(),
                property: property.to_owned(),
                expected: "interpolatable values",
            });
        }
        let key = PropertyKey {
            node: id,
            property: name,
        };
        self.physics.remove(&key);
        self.properties.batch(|graph| {
            graph.write(slot.current, from.clone())?;
            graph.write(slot.target, to.clone())?;
            Ok(())
        })?;
        if behavior.duration == Duration::ZERO {
            self.properties.write(slot.current, to)?;
            self.animations.remove(&key);
        } else {
            self.animations.insert(
                key,
                Animation {
                    from,
                    to,
                    initial_velocity: Velocity::Number(0.0),
                    preserve_velocity: false,
                    elapsed: Duration::ZERO,
                    behavior,
                },
            );
        }
        Ok(())
    }

    /// Installs or removes physics-driven motion on a numeric property.
    pub fn set_physics(
        &mut self,
        node: NodeHandle,
        property: &str,
        physics: Option<Physics>,
    ) -> Result<(), SceneError> {
        let id = self.live(node)?;
        let (name, slot) = self.nodes[id]
            .properties
            .get_key_value(property)
            .ok_or_else(|| SceneError::UnknownProperty {
                element: self.nodes[id].element.name(),
                property: property.to_owned(),
            })?;
        if !matches!(slot.kind, PropertyType::Number) {
            return Err(SceneError::InvalidPropertyType {
                element: self.nodes[id].element.name(),
                property: property.to_owned(),
                expected: "numeric property",
            });
        }
        let key = PropertyKey {
            node: id,
            property: name,
        };
        if let Some(physics) = physics {
            validate_physics(physics).map_err(SceneError::Reactive)?;
            self.physics_specs.insert(key, physics);
            self.behaviors.remove(&key);
            self.animations.remove(&key);
        } else {
            self.physics_specs.remove(&key);
            self.physics.remove(&key);
        }
        Ok(())
    }

    /// Advances every active behavior without invoking Lua.
    pub fn tick_animations(&mut self, delta: Duration) -> Result<AnimationFrame, SceneError> {
        let mut frame = AnimationFrame::default();
        let keys: Vec<_> = self.animations.keys().copied().collect();
        let mut finished = Vec::new();
        for key in keys {
            let animation = self
                .animations
                .get_mut(&key)
                .expect("animation key vanished");
            animation.elapsed = animation.elapsed.saturating_add(delta);
            let complete = animation.elapsed >= animation.behavior.duration;
            let value = if complete {
                animation.to.clone()
            } else {
                animation.value()
            };
            let Some(node) = self.nodes.get(key.node) else {
                finished.push(key);
                continue;
            };
            let slot = node.properties[key.property];
            self.properties.write(slot.current, value)?;
            frame.changes.push(AnimatedChange {
                node: NodeHandle(key.node),
                property: key.property,
                class: property_class(key.property),
            });
            if complete {
                finished.push(key);
            }
        }
        for key in finished {
            self.animations.remove(&key);
        }
        let physics_keys: Vec<_> = self.physics.keys().copied().collect();
        let mut physics_finished = Vec::new();
        for key in physics_keys {
            let Some(node) = self.nodes.get(key.node) else {
                physics_finished.push(key);
                continue;
            };
            let slot = node.properties[key.property];
            let Value::Number(mut current) = *self.properties.read(slot.current)? else {
                physics_finished.push(key);
                continue;
            };
            let motion = self.physics.get_mut(&key).expect("physics key vanished");
            let settled = advance_physics(motion, &mut current, delta);
            self.properties
                .write(slot.current, Value::Number(current))?;
            frame.changes.push(AnimatedChange {
                node: NodeHandle(key.node),
                property: key.property,
                class: property_class(key.property),
            });
            if settled {
                physics_finished.push(key);
            }
        }
        for key in physics_finished {
            self.physics.remove(&key);
        }
        let report = self.properties.flush()?;
        if let Some(error) = report.errors.first() {
            return Err(SceneError::Reactive(format!(
                "{}: {}",
                error.effect, error.message
            )));
        }
        frame.active = !self.animations.is_empty() || !self.physics.is_empty();
        Ok(frame)
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

    /// Reads a boolean current property.
    pub fn bool_value(&self, node: NodeHandle, property: &str) -> Result<bool, SceneError> {
        match self.current(node, property)? {
            Value::Bool(value) => Ok(*value),
            _ => Err(SceneError::Reactive(format!(
                "property `{property}` is not boolean"
            ))),
        }
    }

    /// Reads a color current property.
    pub fn color_value(&self, node: NodeHandle, property: &str) -> Result<Color, SceneError> {
        match self.current(node, property)? {
            Value::Color(value) => Ok(*value),
            _ => Err(SceneError::Reactive(format!(
                "property `{property}` is not a color"
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

impl Animation {
    fn progress(&self) -> f64 {
        let duration = self.behavior.duration.as_secs_f64();
        if duration == 0.0 {
            1.0
        } else {
            (self.elapsed.as_secs_f64() / duration).clamp(0.0, 1.0)
        }
    }

    fn value(&self) -> Value {
        let progress = self.progress();
        if self.preserve_velocity {
            interpolate_hermite(
                &self.from,
                &self.to,
                self.initial_velocity,
                self.behavior.duration.as_secs_f64(),
                progress,
            )
        } else {
            interpolate(&self.from, &self.to, self.behavior.easing.sample(progress))
        }
    }

    fn velocity(&self) -> Velocity {
        let duration = self.behavior.duration.as_secs_f64();
        if duration == 0.0 {
            return zero_velocity(&self.to);
        }
        let progress = self.progress();
        let epsilon = (1.0 / (duration * 1_000.0)).clamp(1e-6, 1e-3);
        let before = (progress - epsilon).max(0.0);
        let after = (progress + epsilon).min(1.0);
        let span = (after - before) * duration;
        let before_value = if self.preserve_velocity {
            interpolate_hermite(
                &self.from,
                &self.to,
                self.initial_velocity,
                duration,
                before,
            )
        } else {
            interpolate(&self.from, &self.to, self.behavior.easing.sample(before))
        };
        let after_value = if self.preserve_velocity {
            interpolate_hermite(&self.from, &self.to, self.initial_velocity, duration, after)
        } else {
            interpolate(&self.from, &self.to, self.behavior.easing.sample(after))
        };
        value_velocity(&before_value, &after_value, span)
    }
}

fn interpolatable(from: &Value, to: &Value) -> bool {
    matches!(
        (from, to),
        (Value::Number(_), Value::Number(_)) | (Value::Color(_), Value::Color(_))
    )
}

fn validate_physics(physics: Physics) -> Result<(), String> {
    match physics {
        Physics::Spring {
            mass,
            damping,
            stiffness,
            epsilon,
        } if mass.is_finite()
            && mass > 0.0
            && damping.is_finite()
            && damping >= 0.0
            && stiffness.is_finite()
            && stiffness > 0.0
            && epsilon.is_finite()
            && epsilon > 0.0 =>
        {
            Ok(())
        }
        Physics::Smoothed { velocity } if velocity.is_finite() && velocity > 0.0 => Ok(()),
        Physics::Spring { .. } => Err("spring values must be finite and physically valid".into()),
        Physics::Smoothed { .. } => {
            Err("smoothed velocity must be finite and greater than zero".into())
        }
    }
}

fn advance_physics(motion: &mut PhysicsAnimation, current: &mut f64, delta: Duration) -> bool {
    let seconds = delta.as_secs_f64();
    match motion.spec {
        Physics::Spring {
            mass,
            damping,
            stiffness,
            epsilon,
        } => {
            let steps = (seconds / (1.0 / 120.0)).ceil().max(1.0) as usize;
            let step = seconds / steps as f64;
            for _ in 0..steps {
                let acceleration =
                    (stiffness * (motion.target - *current) - damping * motion.velocity) / mass;
                motion.velocity += acceleration * step;
                *current += motion.velocity * step;
            }
            if (*current - motion.target).abs() <= epsilon && motion.velocity.abs() <= epsilon {
                *current = motion.target;
                motion.velocity = 0.0;
                true
            } else {
                false
            }
        }
        Physics::Smoothed { velocity } => {
            let distance = motion.target - *current;
            let step = velocity * seconds;
            if distance.abs() <= step {
                *current = motion.target;
                motion.velocity = 0.0;
                true
            } else {
                motion.velocity = velocity.copysign(distance);
                *current += motion.velocity * seconds;
                false
            }
        }
    }
}

fn zero_velocity(value: &Value) -> Velocity {
    match value {
        Value::Color(_) => Velocity::Color([0.0; 4]),
        _ => Velocity::Number(0.0),
    }
}

fn value_velocity(from: &Value, to: &Value, seconds: f64) -> Velocity {
    let seconds = seconds.max(f64::EPSILON);
    match (from, to) {
        (Value::Number(from), Value::Number(to)) => Velocity::Number((to - from) / seconds),
        (Value::Color(from), Value::Color(to)) => Velocity::Color([
            (to.red as f64 - from.red as f64) / seconds,
            (to.green as f64 - from.green as f64) / seconds,
            (to.blue as f64 - from.blue as f64) / seconds,
            (to.alpha as f64 - from.alpha as f64) / seconds,
        ]),
        _ => Velocity::Number(0.0),
    }
}

fn interpolate(from: &Value, to: &Value, progress: f64) -> Value {
    match (from, to) {
        (Value::Number(from), Value::Number(to)) => Value::Number(from + (to - from) * progress),
        (Value::Color(from), Value::Color(to)) => Value::Color(Color {
            red: (from.red as f64 + (to.red as f64 - from.red as f64) * progress) as f32,
            green: (from.green as f64 + (to.green as f64 - from.green as f64) * progress) as f32,
            blue: (from.blue as f64 + (to.blue as f64 - from.blue as f64) * progress) as f32,
            alpha: (from.alpha as f64 + (to.alpha as f64 - from.alpha as f64) * progress) as f32,
        }),
        _ => to.clone(),
    }
}

fn interpolate_hermite(
    from: &Value,
    to: &Value,
    velocity: Velocity,
    duration: f64,
    progress: f64,
) -> Value {
    let t2 = progress * progress;
    let t3 = t2 * progress;
    let from_weight = 2.0 * t3 - 3.0 * t2 + 1.0;
    let velocity_weight = t3 - 2.0 * t2 + progress;
    let to_weight = -2.0 * t3 + 3.0 * t2;
    match (from, to, velocity) {
        (Value::Number(from), Value::Number(to), Velocity::Number(velocity)) => Value::Number(
            from_weight * from + velocity_weight * duration * velocity + to_weight * to,
        ),
        (Value::Color(from), Value::Color(to), Velocity::Color(velocity)) => {
            let from = [from.red, from.green, from.blue, from.alpha].map(f64::from);
            let to = [to.red, to.green, to.blue, to.alpha].map(f64::from);
            let channels: [f32; 4] = std::array::from_fn(|index| {
                (from_weight * from[index]
                    + velocity_weight * duration * velocity[index]
                    + to_weight * to[index])
                    .clamp(0.0, 1.0) as f32
            });
            Value::Color(Color {
                red: channels[0],
                green: channels[1],
                blue: channels[2],
                alpha: channels[3],
            })
        }
        _ => interpolate(from, to, progress),
    }
}

fn property_class(property: &str) -> PropertyClass {
    match property {
        "x" | "y" | "scale" | "rotation" | "opacity" | "transition_x" | "transition_y" => {
            PropertyClass::Transform
        }
        "color" | "radius" | "border_width" | "border_color" => PropertyClass::Paint,
        _ => PropertyClass::Layout,
    }
}

fn cubic_bezier(progress: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let curve = |t: f64, first: f64, second: f64| {
        let inverse = 1.0 - t;
        3.0 * inverse * inverse * t * first + 3.0 * inverse * t * t * second + t * t * t
    };
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..20 {
        let midpoint = (low + high) / 2.0;
        if curve(midpoint, x1, x2) < progress {
            low = midpoint;
        } else {
            high = midpoint;
        }
    }
    curve((low + high) / 2.0, y1, y2)
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
        number("transition_x", 0.0),
        number("transition_y", 0.0),
        boolean("enabled", true),
    ];
    match element {
        Element::Item | Element::MouseArea => {}
        Element::Rect => {
            properties.extend([
                color("color", Color::rgba8(255, 255, 255, 255)),
                number("radius", 0.0),
                number("border_width", 0.0),
                color("border_color", Color::rgba8(0, 0, 0, 0)),
                number("blur", 0.0),
                color("shadow_color", Color::rgba8(0, 0, 0, 0)),
                number("shadow_blur", 0.0),
                number("shadow_spread", 0.0),
                number("shadow_offset_x", 0.0),
                number("shadow_offset_y", 0.0),
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

    #[test]
    fn behavior_intercepts_writes_and_keeps_target_live() {
        let mut scene = Scene::new();
        let rect = scene.create(Element::Rect);
        scene
            .set_behavior(
                rect,
                "width",
                Some(Behavior {
                    duration: Duration::from_millis(200),
                    easing: Easing::Linear,
                }),
            )
            .unwrap();

        scene.assign(rect, "width", 100.0).unwrap();
        assert_eq!(scene.current(rect, "width").unwrap(), &Value::Number(0.0));
        assert_eq!(scene.target(rect, "width").unwrap(), &Value::Number(100.0));
        let frame = scene.tick_animations(Duration::from_millis(100)).unwrap();

        assert_eq!(scene.current(rect, "width").unwrap(), &Value::Number(50.0));
        assert_eq!(frame.changes[0].class, PropertyClass::Layout);
        assert!(frame.active);
    }

    #[test]
    fn interrupted_animation_retargets_without_a_jump() {
        let mut scene = Scene::new();
        let rect = scene.create(Element::Rect);
        let behavior = Behavior {
            duration: Duration::from_millis(200),
            easing: Easing::Linear,
        };
        scene.set_behavior(rect, "opacity", Some(behavior)).unwrap();
        scene.assign(rect, "opacity", 0.0).unwrap();
        scene.tick_animations(Duration::from_millis(50)).unwrap();
        let before = scene.number(rect, "opacity").unwrap();

        scene.assign(rect, "opacity", 0.8).unwrap();
        let retargeted = scene.number(rect, "opacity").unwrap();
        scene.tick_animations(Duration::from_millis(1)).unwrap();
        let after = scene.number(rect, "opacity").unwrap();

        assert_eq!(before, retargeted);
        assert!((after - before).abs() < 0.02);
        assert_eq!(scene.target(rect, "opacity").unwrap(), &Value::Number(0.8));
    }

    #[test]
    fn paint_animation_finishes_at_the_exact_target() {
        let mut scene = Scene::new();
        let rect = scene.create(Element::Rect);
        scene
            .set_behavior(
                rect,
                "color",
                Some(Behavior {
                    duration: Duration::from_millis(120),
                    easing: Easing::OutCubic,
                }),
            )
            .unwrap();
        scene.assign(rect, "color", "#7c3aed").unwrap();

        let frame = scene.tick_animations(Duration::from_millis(120)).unwrap();

        assert_eq!(scene.current(rect, "color"), scene.target(rect, "color"));
        assert_eq!(frame.changes[0].class, PropertyClass::Paint);
        assert!(!frame.active);
    }

    #[test]
    fn spring_retargets_with_continuous_position_and_velocity() {
        let mut scene = Scene::new();
        let item = scene.create(Element::Item);
        scene
            .set_physics(
                item,
                "x",
                Some(Physics::Spring {
                    mass: 1.0,
                    damping: 18.0,
                    stiffness: 180.0,
                    epsilon: 0.001,
                }),
            )
            .unwrap();
        scene.assign(item, "x", 100.0).unwrap();
        scene.tick_animations(Duration::from_millis(80)).unwrap();
        let before = scene.number(item, "x").unwrap();

        scene.assign(item, "x", -20.0).unwrap();
        assert_eq!(scene.number(item, "x").unwrap(), before);
        scene.tick_animations(Duration::from_millis(1)).unwrap();
        assert!((scene.number(item, "x").unwrap() - before).abs() < 2.0);
    }

    #[test]
    fn smoothed_motion_obeys_velocity_limit() {
        let mut scene = Scene::new();
        let item = scene.create(Element::Item);
        scene
            .set_physics(item, "x", Some(Physics::Smoothed { velocity: 200.0 }))
            .unwrap();
        scene.assign(item, "x", 100.0).unwrap();

        let frame = scene.tick_animations(Duration::from_millis(100)).unwrap();
        assert_eq!(scene.number(item, "x").unwrap(), 20.0);
        assert!(frame.active);
        scene.tick_animations(Duration::from_millis(400)).unwrap();
        assert_eq!(scene.number(item, "x").unwrap(), 100.0);
    }
}
