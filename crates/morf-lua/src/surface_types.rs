use luna::{Context, Value as LuaValue};

use morf_region::Region;
use morf_scene::{Behavior, NodeHandle, Value as SceneValue};

/// Edges used to anchor a configured layer surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceAnchors {
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub left: bool,
}

impl Default for SurfaceAnchors {
    fn default() -> Self {
        Self {
            top: true,
            right: true,
            bottom: false,
            left: true,
        }
    }
}

/// Compositor space reserved along each output edge by a dedicated surface.
///
/// A layer surface can only reserve space on an unambiguously anchored edge, so
/// a frame drawn on all four edges cannot reserve for itself. These thicknesses
/// ask the engine for one zero-size reserver surface per non-zero edge, which is
/// what shrinks the tiling area.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SurfaceReserve {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

impl SurfaceReserve {
    /// Returns whether any edge asks for reserved space.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Returns the requested thickness per edge, in anchor order.
    pub fn edges(&self) -> [(&'static str, u32); 4] {
        [
            ("top", self.top),
            ("right", self.right),
            ("bottom", self.bottom),
            ("left", self.left),
        ]
    }
}

/// Native layer-surface settings assigned by Lua before startup.
///
/// Not `Eq`: a region carries its shape's parameters, and those are floats.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerSurfaceConfig {
    pub namespace: String,
    pub width: u32,
    pub height: u32,
    pub exclusive_zone: i32,
    pub anchors: SurfaceAnchors,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    pub layer: String,
    pub keyboard_focus: String,
    pub input_regions: Option<Vec<Region>>,
    pub reserve: SurfaceReserve,
    /// Whether the exclusive zone follows the surface's own size on its
    /// anchored edge, rather than being a number the configuration keeps in
    /// step by hand. A bar that grows should push windows with it.
    pub exclusive_auto: bool,
    /// Whether the whole surface is opaque, so the compositor can skip
    /// blending whatever is behind it. False by default, because a bar with a
    /// transparent corner that claims otherwise draws garbage there.
    pub opaque: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PopupConstraintConfig {
    pub slide_x: bool,
    pub slide_y: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    pub resize_x: bool,
    pub resize_y: bool,
}

impl Default for PopupConstraintConfig {
    fn default() -> Self {
        Self {
            slide_x: true,
            slide_y: true,
            flip_x: true,
            flip_y: true,
            resize_x: false,
            resize_y: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopupSurfaceConfig {
    pub parent: Option<u64>,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub anchor_width: i32,
    pub anchor_height: i32,
    pub width: u32,
    pub height: u32,
    pub anchor_edge: String,
    pub gravity: String,
    pub offset_x: i32,
    pub offset_y: i32,
    pub constraints: PopupConstraintConfig,
    pub grab_focus: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FloatingSurfaceConfig {
    pub parent: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub minimum_width: u32,
    pub minimum_height: u32,
    pub maximum_width: Option<u32>,
    pub maximum_height: Option<u32>,
    pub title: String,
    pub app_id: String,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WindowSurfaceKind {
    Popup(PopupSurfaceConfig),
    Floating(FloatingSurfaceConfig),
    /// One additional wlr-layer-shell surface beyond the shell's own.
    Layer(LayerSurfaceConfig),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSurfaceConfig {
    pub id: u64,
    pub root: NodeHandle,
    pub visible: bool,
    pub updates_enabled: bool,
    pub kind: WindowSurfaceKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowSurfaceAction {
    Move { id: u64 },
    Resize { id: u64, edge: String },
}

impl Default for LayerSurfaceConfig {
    fn default() -> Self {
        Self {
            namespace: "morf".to_owned(),
            width: 0,
            height: 32,
            exclusive_zone: 32,
            anchors: SurfaceAnchors::default(),
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            layer: "top".to_owned(),
            keyboard_focus: "on_demand".to_owned(),
            input_regions: None,
            reserve: SurfaceReserve::default(),
            exclusive_auto: false,
            opaque: false,
        }
    }
}

/// Deferred parent and anchor transition requested by Lua.
#[derive(Clone, Debug)]
pub struct ParentTransitionRequest {
    pub node: NodeHandle,
    pub parent: NodeHandle,
    pub anchors: Option<std::collections::BTreeMap<String, SceneValue>>,
    pub behavior: Behavior,
}

/// Primitive value accepted by the bounded IPC surface.
#[derive(Clone, Debug, PartialEq)]
pub enum IpcValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
}

/// Deferred virtual keyboard request produced by Lua.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualKeyboardRequest {
    /// One evdev keycode state change.
    Key { keycode: u32, pressed: bool },
    /// XKB modifier masks and layout group.
    Modifiers {
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    },
}

/// Deferred input-method-v2 request produced by Lua.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputMethodRequest {
    /// Inserts committed UTF-8 text.
    Commit(String),
    /// Replaces the preedit string and cursor range.
    Preedit { text: String, begin: i32, end: i32 },
    /// Deletes byte ranges around the cursor.
    Delete { before: u32, after: u32 },
}

/// Deferred text-input-v3 state request produced by Lua.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextInputRequest {
    Disable,
    Surrounding {
        text: String,
        cursor: i32,
        anchor: i32,
    },
    ContentType {
        hints: u32,
        purpose: u32,
    },
    CursorRect {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
}

/// One compositor output capture delivered to Lua.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Screencopy {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Bytes between adjacent rows.
    pub stride: u32,
    /// Shared-memory pixel format name.
    pub format: String,
    /// Whether rows are ordered bottom-to-top.
    pub y_invert: bool,
    /// Whether the picture is on the GPU, with `pixels` empty.
    pub gpu: bool,
    /// A source string `ui.Image` resolves, holding this capture's pixels.
    ///
    /// The point of it: a configuration that wants the picture on screen sets
    /// `ui.Image { source = frame.source }` and is done. Without this the
    /// capture protocols hand over bytes with nowhere to go — `ui.Image`
    /// resolves paths, so showing one meant encoding a file and reading it
    /// back, megabytes per thumbnail per refresh to move pixels already in
    /// memory.
    ///
    /// Named after the request, so a thumbnail that refreshes replaces itself.
    /// `pixels` is still there for a configuration that wants the bytes.
    pub source: String,
    /// Captured bytes including stride padding.
    pub pixels: Vec<u8>,
}

/// Correlated capture request queued by Lua.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreencopyRequest {
    /// Runtime-local request identifier.
    pub id: u64,
    /// Whether the compositor should include the cursor image.
    pub include_cursor: bool,
    /// A window to capture instead of the output, by the identifier
    /// `morf.windows` reported.
    ///
    /// By identifier rather than index or title: an index means something else
    /// the moment a window opens, and two windows of one application share a
    /// title as readily as an app id.
    pub window: Option<String>,
    /// Whether the picture should stay on the GPU.
    ///
    /// The compositor then draws into memory the renderer exported, and the
    /// frame's `source` is a texture rather than pixels: nothing is copied
    /// out and nothing uploaded back. Honoured where the compositor and the
    /// GPU allow it, and quietly shared memory where they do not.
    pub gpu: bool,
}

impl IpcValue {
    pub(crate) fn to_lua<'gc>(&self, ctx: Context<'gc>) -> LuaValue<'gc> {
        match self {
            Self::Nil => LuaValue::Nil,
            Self::Boolean(value) => LuaValue::Boolean(*value),
            Self::Integer(value) => LuaValue::Integer(*value),
            Self::Number(value) => LuaValue::Number(*value),
            Self::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
        }
    }

    pub(crate) fn from_lua(value: LuaValue<'_>) -> Result<Self, String> {
        match value {
            LuaValue::Nil => Ok(Self::Nil),
            LuaValue::Boolean(value) => Ok(Self::Boolean(value)),
            LuaValue::Integer(value) => Ok(Self::Integer(value)),
            LuaValue::Number(value) if value.is_finite() => Ok(Self::Number(value)),
            LuaValue::String(value) => Ok(Self::String(value.display_lossy().to_string())),
            value => Err(format!(
                "values crossing the Lua boundary must be nil, boolean, number, or string, found {}",
                value.type_name()
            )),
        }
    }

    /// The same value as the scene stores it.
    ///
    /// This lived on a second enum with the same five variants and the same
    /// three conversions, which existed only because the reactive graph and the
    /// IPC surface had each grown one — along with a pair of shims to carry a
    /// value from one to the other.
    pub(crate) fn to_scene(&self) -> SceneValue {
        match self {
            Self::Nil => SceneValue::Nil,
            Self::Boolean(value) => SceneValue::Bool(*value),
            Self::Integer(value) => SceneValue::Number(*value as f64),
            Self::Number(value) => SceneValue::Number(*value),
            Self::String(value) => SceneValue::String(value.clone()),
        }
    }
}
