use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::xdg::window::Window;
use std::error::Error as StdError;
use std::fmt;
use wayland_client::protocol::wl_surface;
use wayland_client::{Connection, EventQueue};
use wayland_protocols::xdg::shell::client::xdg_toplevel;

use crate::{state_types::*, types::*};

/// Geometry and identity for an xdg toplevel surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FloatingConfig {
    /// Initial logical width.
    pub width: u32,
    /// Initial logical height.
    pub height: u32,
    /// Smallest compositor-configured logical width.
    pub minimum_width: u32,
    /// Smallest compositor-configured logical height.
    pub minimum_height: u32,
    /// Largest compositor-configured logical width when bounded.
    pub maximum_width: Option<u32>,
    /// Largest compositor-configured logical height when bounded.
    pub maximum_height: Option<u32>,
    /// Compositor-visible title.
    pub title: String,
    /// Desktop application identifier.
    pub app_id: String,
    /// Requests initial minimized state.
    pub minimized: bool,
    /// Requests initial maximized state.
    pub maximized: bool,
    /// Requests initial fullscreen state.
    pub fullscreen: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloatingResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl FloatingResizeEdge {
    pub(crate) fn protocol(self) -> xdg_toplevel::ResizeEdge {
        match self {
            Self::Top => xdg_toplevel::ResizeEdge::Top,
            Self::Bottom => xdg_toplevel::ResizeEdge::Bottom,
            Self::Left => xdg_toplevel::ResizeEdge::Left,
            Self::Right => xdg_toplevel::ResizeEdge::Right,
            Self::TopLeft => xdg_toplevel::ResizeEdge::TopLeft,
            Self::TopRight => xdg_toplevel::ResizeEdge::TopRight,
            Self::BottomLeft => xdg_toplevel::ResizeEdge::BottomLeft,
            Self::BottomRight => xdg_toplevel::ResizeEdge::BottomRight,
        }
    }
}

/// Compositor output power state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputPowerMode {
    /// The output is powered down.
    Off,
    /// The output is powered on.
    On,
}

/// Atomically committed input-method context.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputMethodState {
    /// Whether a focused text input requested this input method.
    pub active: bool,
    /// UTF-8 text around the application cursor when supported.
    pub surrounding_text: Option<String>,
    /// Byte offset of the cursor in surrounding text.
    pub cursor: u32,
    /// Byte offset of the selection anchor in surrounding text.
    pub anchor: u32,
    /// Number of compositor done events received.
    pub serial: u32,
}

/// Atomically committed text-input-v3 edit batch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextInputState {
    /// Whether this client's surface has text-input focus.
    pub focused: bool,
    /// Current preedit string when changed in this batch.
    pub preedit: Option<String>,
    /// Preedit cursor start in bytes when supplied.
    pub preedit_begin: i32,
    /// Preedit cursor end in bytes when supplied.
    pub preedit_end: i32,
    /// UTF-8 text committed by the input method.
    pub commit: Option<String>,
    /// Bytes to delete before the cursor.
    pub delete_before: u32,
    /// Bytes to delete after the cursor.
    pub delete_after: u32,
    /// Serial supplied by the compositor done event.
    pub serial: u32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            namespace: "morf".to_owned(),
            width: 0,
            height: 32,
            exclusive_zone: 32,
            output: None,
            anchors: LayerAnchors::default(),
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            layer: ShellLayer::default(),
            keyboard_focus: KeyboardFocus::default(),
        }
    }
}

/// Surface category associated with an input event.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceRole {
    /// One wlr-layer-shell surface, addressed by its client-local identifier.
    Layer(u64),
    Popup(u64),
    Floating(u64),
}

/// Event produced by the layer-surface connection.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerEvent {
    /// A popup or floating window's own scale changed.
    ///
    /// Separate from `Scale`, which is a layer surface's, because the two are
    /// addressed differently and a caller resizes different things for each.
    AuxScale { role: SurfaceRole, scale_120: u32 },
    /// The compositor selected a logical size for one layer surface.
    Configure { id: u64, width: u32, height: u32 },
    /// One layer surface's preferred scale changed in protocol-native 120ths.
    Scale { id: u64, scale_120: u32 },
    /// The compositor permits the next animation and paint tick.
    Frame { id: u64, time_ms: u32 },
    /// The pointer moved over or entered the surface.
    PointerMotion {
        surface: SurfaceRole,
        x: f64,
        y: f64,
    },
    /// The pointer left the surface.
    PointerLeave { surface: SurfaceRole },
    /// A pointer button changed state.
    PointerButton {
        surface: SurfaceRole,
        button: u32,
        pressed: bool,
        x: f64,
        y: f64,
    },
    /// A pointer wheel or touchpad axis changed.
    PointerAxis {
        surface: SurfaceRole,
        x: f64,
        y: f64,
        horizontal: f64,
        vertical: f64,
        horizontal_steps: i32,
        vertical_steps: i32,
    },
    /// A touch contact began on the surface.
    TouchDown {
        surface: SurfaceRole,
        id: i32,
        x: f64,
        y: f64,
    },
    /// A touch contact moved on the surface.
    TouchMotion {
        surface: SurfaceRole,
        id: i32,
        x: f64,
        y: f64,
    },
    /// A touch contact ended on the surface.
    TouchUp {
        surface: SurfaceRole,
        id: i32,
        x: f64,
        y: f64,
    },
    /// The compositor cancelled every active touch contact.
    TouchCancel,
    /// A keyboard key changed state.
    Key {
        surface: SurfaceRole,
        keysym: u32,
        text: Option<String>,
        pressed: bool,
        repeat: bool,
    },
    /// A configured seat idle threshold changed state.
    Idle { timeout_ms: u32, idle: bool },
    /// The compositor clipboard selection changed.
    Clipboard { text: Option<String> },
    /// An output capture completed or failed.
    Screencopy {
        /// Runtime-local request identifier.
        request_id: u64,
        /// Captured pixels or compositor failure.
        result: Result<ScreencopyFrame, String>,
    },
    /// A focused text input committed a new input-method context.
    InputMethod(InputMethodState),
    /// An input method committed edits for this client's text input.
    TextInput(TextInputState),
    /// The compositor output set changed.
    Screens(Vec<ScreenInfo>),
    /// The compositor positioned and sized the popup.
    PopupConfigure { id: u64, width: u32, height: u32 },
    /// The compositor permits the next popup paint tick.
    PopupFrame { id: u64, time_ms: u32 },
    /// The compositor dismissed the popup.
    PopupDone { id: u64 },
    /// The compositor configured the floating window.
    FloatingConfigure { id: u64, width: u32, height: u32 },
    /// The compositor permits the next floating-window paint tick.
    FloatingFrame { id: u64, time_ms: u32 },
    /// The compositor requested that the floating window close.
    FloatingClose { id: u64 },
    /// The compositor accepted exclusive session ownership.
    SessionLocked,
    /// The compositor rejected or ended the session lock.
    SessionLockFinished,
    /// One output lock surface received its logical size.
    SessionLockConfigure {
        index: usize,
        width: u32,
        height: u32,
    },
    /// One output and its lock surface were removed.
    SessionLockSurfaceRemoved { index: usize },
    /// The compositor permits the next lock-surface paint tick.
    SessionLockFrame { index: usize, time_ms: u32 },
    /// The compositor closed one layer surface.
    Closed { id: u64 },
}

/// Wayland connection or protocol setup failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaylandError(pub(crate) String);

impl fmt::Display for WaylandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for WaylandError {}

/// Live layer surface and its event queue.
pub struct LayerClient {
    pub(crate) connection: Connection,
    pub(crate) queue: EventQueue<LayerState>,
    pub(crate) state: LayerState,
}

/// The shell role a primary surface is actually wearing.
///
/// morf wants a layer surface: it is the protocol built for shells, and it is
/// what gives an anchored bar, an exclusive zone and keyboard focus that a
/// toplevel cannot ask for. But `wlr-layer-shell` is an optional extension, and
/// kiosk compositors do not carry it — `cage`, which is what greetd runs a
/// greeter inside, offers only `xdg-shell`. Making the bind fatal meant morf
/// refused to start there at all, which is the wrong trade: a greeter that
/// covers the screen is precisely the case where a fullscreen toplevel is
/// indistinguishable from the layer surface it is standing in for, because the
/// compositor gives its single client the whole output regardless.
///
/// So the layer role is preferred and the toplevel is the fallback. The
/// fallback is worth less than it looks on a general-purpose compositor —
/// anchors, margins and the exclusive zone have no meaning for a toplevel and
/// are dropped — which is why it is only ever reached when the compositor has
/// no layer-shell whatsoever, leaving nothing better to fall back from.
pub(crate) enum ShellSurface {
    /// A `wlr-layer-shell` surface: what a shell wants.
    Layer(LayerSurface),
    /// A fullscreen xdg toplevel, standing in where there is no layer-shell.
    Window(Box<Window>),
}

impl ShellSurface {
    /// The underlying `wl_surface`, whichever role wraps it.
    pub(crate) fn wl_surface(&self) -> &wl_surface::WlSurface {
        match self {
            Self::Layer(layer) => layer.wl_surface(),
            Self::Window(window) => window.wl_surface(),
        }
    }

    /// The layer surface, when this really is one.
    ///
    /// Callers use this for the things only layer-shell can do — re-anchoring,
    /// the exclusive zone, parenting a popup — and skip them otherwise, since a
    /// toplevel has no equivalent to skip *to*.
    pub(crate) fn as_layer(&self) -> Option<&LayerSurface> {
        match self {
            Self::Layer(layer) => Some(layer),
            Self::Window(_) => None,
        }
    }

    /// Commits pending surface state.
    pub(crate) fn commit(&self) {
        match self {
            Self::Layer(layer) => layer.commit(),
            Self::Window(window) => window.commit(),
        }
    }
}
