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
    /// Tessellated SVG path primitive.
    Shape,
    /// Pointer and focus event target with no visual output.
    MouseArea,
    /// Sequential horizontal positioner.
    Row,
    /// Sequential vertical positioner.
    Column,
    /// Fixed-column two-dimensional positioner.
    Grid,
    /// Horizontal positioner honoring attached layout constraints.
    RowLayout,
    /// Vertical positioner honoring attached layout constraints.
    ColumnLayout,
    /// Two-dimensional positioner honoring attached layout constraints.
    GridLayout,
    /// Clipped viewport over movable content.
    Flickable,
    /// Non-painting container for a lazily constructed child.
    Loader,
    /// Non-painting periodic callback object.
    Timer,
}

impl Element {
    fn name(self) -> &'static str {
        match self {
            Self::Item => "Item",
            Self::Inset => "Inset",
            Self::Rect => "Rect",
            Self::ClipRect => "ClipRect",
            Self::Text => "Text",
            Self::Image => "Image",
            Self::Icon => "Icon",
            Self::Shape => "Shape",
            Self::MouseArea => "MouseArea",
            Self::Row => "Row",
            Self::Column => "Column",
            Self::Grid => "Grid",
            Self::RowLayout => "RowLayout",
            Self::ColumnLayout => "ColumnLayout",
            Self::GridLayout => "GridLayout",
            Self::Flickable => "Flickable",
            Self::Loader => "Loader",
            Self::Timer => "Timer",
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

