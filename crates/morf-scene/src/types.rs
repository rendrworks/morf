use morf_reactive::{Graph, SignalId};
use slotmap::{SlotMap, new_key_type};
use std::collections::{BTreeMap, HashMap};

use crate::{animation::*, groups::*, hashing::*};

new_key_type! {
    pub(crate) struct NodeId;
}

/// A generational scene node handle safe to retain outside the arena.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NodeHandle(pub(crate) NodeId);

impl NodeHandle {
    pub(crate) fn id(self) -> NodeId {
        self.0
    }
}

/// Element kinds implemented by the first scene milestone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Element {
    /// Non-painting container.
    Item,
    /// Single-child container applying configurable margins.
    Inset,
    /// Rounded rectangle primitive.
    Rect,
    /// Rounded rectangle that clips content and overlays its border.
    ClipRect,
    /// Shaped text primitive.
    Text,
    /// Raster or SVG image primitive.
    Image,
    /// XDG icon-theme image primitive.
    Icon,
    /// Signed-distance field composed from its `SdfShape` children.
    Sdf,
    /// One analytic distance field inside an [`Element::Sdf`].
    ///
    /// Never painted on its own: the parent reads its geometry, its shape and
    /// its combining operation, and resolves the whole composition in one
    /// fragment shader. Because the layer is an ordinary node, every number it
    /// carries animates through the same behaviors as any other property.
    SdfShape,
    /// Pointer and focus event target with no visual output.
    MouseArea,
    /// Sequential horizontal positioner.
    Row,
    /// Sequential vertical positioner.
    Column,
    /// Fixed-column two-dimensional positioner.
    Grid,
    /// Clipped viewport over movable content.
    Flickable,
    /// Non-painting container for a lazily constructed child.
    Loader,
    /// Non-painting periodic callback object.
    Timer,
    /// A flexbox container: its children are placed by grow, shrink, basis,
    /// wrap and alignment rather than by their own `x` and `y`.
    Flex,
    /// A container whose measure and placement are functions the
    /// configuration wrote.
    Custom,
}

impl Element {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Item => "Item",
            Self::Inset => "Inset",
            Self::Rect => "Rect",
            Self::ClipRect => "ClipRect",
            Self::Text => "Text",
            Self::Image => "Image",
            Self::Icon => "Icon",
            Self::Sdf => "Sdf",
            Self::SdfShape => "SdfShape",
            Self::MouseArea => "MouseArea",
            Self::Row => "Row",
            Self::Column => "Column",
            Self::Grid => "Grid",
            Self::Flickable => "Flickable",
            Self::Loader => "Loader",
            Self::Timer => "Timer",
            Self::Flex => "Flex",
            Self::Custom => "Layout",
        }
    }
}

/// sRGB-encoded RGBA colour with components in the inclusive zero-to-one range.
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

    /// Reads a colour from any form a configuration writes: hex with or
    /// without `#`, `0x`, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`,
    /// `oklab()`, `oklch()`, `gray()`, `transparent`, and the CSS names.
    ///
    /// Returns `None` rather than a fallback so a typo in a colour surfaces as
    /// an error at the property that used it.
    pub fn parse(input: &str) -> Option<Self> {
        crate::color::parse(input)
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
    pub(crate) nodes: SlotMap<NodeId, Node>,
    /// Shaders attached to nodes, by node.
    ///
    /// A side table rather than node properties: property names are `&'static
    /// str`, so a per-shader parameter name would have to be leaked, and giving
    /// every element a fixed set of numbered slots would make every rectangle
    /// in the scene carry two signals per slot whether or not it has a shader.
    /// A shader is rare; it should cost nothing when absent.
    pub(crate) shaders: FastMap<NodeId, NodeShader>,
    pub(crate) properties: Graph<Value>,
    pub(crate) behaviors: FastMap<PropertyKey, Behavior>,
    pub(crate) animations: FastMap<PropertyKey, Animation>,
    pub(crate) physics: FastMap<PropertyKey, PhysicsAnimation>,
    pub(crate) physics_specs: FastMap<PropertyKey, Physics>,
    pub(crate) paused_physics: FastSet<PropertyKey>,
    pub(crate) events: Vec<AnimationEvent>,
    pub(crate) groups: HashMap<GroupId, RunningGroup>,
    pub(crate) group_events: Vec<GroupEvent>,
    pub(crate) next_group: u64,
    /// Bumped whenever something that layout reads changes.
    ///
    /// Layout is the most expensive thing a frame does — it walks the whole
    /// tree measuring, resolving anchors and placing children — and most frames
    /// change nothing it reads. A colour easing, a morph advancing, an opacity
    /// fading: none of them move a box. Recording when the geometry last
    /// actually moved lets a paint reuse the layout it already has.
    pub(crate) layout_revision: u64,
    /// Nodes destroyed since anyone last asked.
    ///
    /// Every cache keyed on a node lives outside this crate — shaped text
    /// buffers in `morf-text`, transforms in `morf-lua`, atlases in the GPU
    /// backend — and none of them can see a node die. Without a signal that
    /// crosses the boundary they grow for the life of the process, and each one
    /// grew its own eviction method that nothing ever called. This is that
    /// signal: the scene records what it destroyed and whoever drives the frame
    /// hands the list to everything holding node-keyed state.
    pub(crate) removed: Vec<NodeHandle>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PropertyKey {
    pub(crate) node: NodeId,
    pub(crate) property: &'static str,
}

/// A compiled shader attached to a node, and the values it was given.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeShader {
    /// Which registered program, by the hash of its generated WGSL.
    pub program: u64,
    /// Parameter values, flattened in declaration order.
    pub params: Vec<f32>,
    /// Values for the shader's data blocks, one run per block in binding
    /// order. Read-only to the shader; the configuration owns them.
    pub data: Vec<Vec<f32>>,
    /// Whether the shader reads what is rendered underneath, and so runs in
    /// the composite pass over a layer rather than in the field pass.
    pub samples_behind: bool,
    /// Whether the shader decides its own coverage rather than colouring what
    /// the node's own shape already covered.
    ///
    /// It travels with the attachment because it changes the *geometry* the
    /// fragment stage walks, not just the colour: a shader that owns its
    /// coverage has to be given the node's whole rectangle, or it paints only
    /// where the shape it replaced would have been.
    pub owns_coverage: bool,
}

pub(crate) struct Node {
    pub(crate) element: Element,
    pub(crate) parent: Option<NodeId>,
    // Handles rather than raw ids so the children can be handed out as a
    // borrowed slice. Every tree walk in the engine asks for them — layout does
    // it five times per node, and paint and hit testing once each — so building
    // a fresh Vec per call put hundreds of allocations in every frame.
    pub(crate) children: Vec<NodeHandle>,
    pub(crate) properties: FastMap<&'static str, PropertySlot>,
}

#[derive(Clone, Copy)]
pub(crate) struct PropertySlot {
    pub(crate) current: SignalId,
    pub(crate) target: SignalId,
    pub(crate) kind: PropertyType,
}

#[derive(Clone, Copy)]
pub(crate) enum PropertyType {
    Any,
    Bool,
    Number,
    String,
    Color,
}
