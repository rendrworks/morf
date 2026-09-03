use wayland_protocols::xdg::shell::client::xdg_positioner;

/// Edges used to anchor a layer-shell surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayerAnchors {
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub left: bool,
}

impl Default for LayerAnchors {
    fn default() -> Self {
        Self {
            top: true,
            right: true,
            bottom: false,
            left: true,
        }
    }
}

/// Compositor layer used by a layer-shell surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShellLayer {
    Background,
    Bottom,
    #[default]
    Top,
    Overlay,
}

/// Keyboard focus policy for a layer-shell surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyboardFocus {
    None,
    Exclusive,
    #[default]
    OnDemand,
}

/// Configuration for a layer-shell surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BarConfig {
    /// Surface namespace exposed to the compositor.
    pub namespace: String,
    /// Requested logical width, or zero for compositor-selected width.
    pub width: u32,
    /// Requested logical height.
    pub height: u32,
    /// Layer-shell exclusive zone in logical pixels.
    pub exclusive_zone: i32,
    /// Compositor output name, or all outputs when unset.
    pub output: Option<String>,
    /// Surface edges anchored to the output.
    pub anchors: LayerAnchors,
    /// Logical top margin.
    pub margin_top: i32,
    /// Logical right margin.
    pub margin_right: i32,
    /// Logical bottom margin.
    pub margin_bottom: i32,
    /// Logical left margin.
    pub margin_left: i32,
    /// Compositor layer used by the surface.
    pub layer: ShellLayer,
    /// Keyboard focus policy used by the surface.
    pub keyboard_focus: KeyboardFocus,
}

/// Integer surface-local rectangle used to construct an input region.
///
/// The same four fields `morf_region` already defines, so it is that type
/// rather than a copy of it. Two names for one shape meant a field-by-field
/// rebuild of every rectangle on the way from the region rasteriser to the
/// compositor, allocated fresh on each update.
pub type InputRect = morf_region::Rect;

/// Capability-derived compositor output description.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScreenInfo {
    pub id: u32,
    pub name: Option<String>,
    pub make: String,
    pub model: String,
    pub description: Option<String>,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
    pub physical_size: Option<(i32, i32)>,
    pub scale: i32,
    pub transform: &'static str,
}

/// Pixel encoding returned by a compositor screencopy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreencopyFormat {
    Argb8888,
    Xrgb8888,
}

/// One completed output capture in row-major shared-memory layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreencopyFrame {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Bytes between adjacent rows.
    pub stride: u32,
    /// Pixel channel encoding.
    pub format: ScreencopyFormat,
    /// Whether rows are ordered bottom-to-top.
    pub y_invert: bool,
    /// Captured bytes including stride padding.
    pub pixels: Vec<u8>,
}

/// Geometry for a popup anchored to a layer surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PopupConfig {
    /// Parent-surface rectangle used as the popup anchor.
    pub anchor: InputRect,
    /// Requested popup width in logical pixels.
    pub width: u32,
    /// Requested popup height in logical pixels.
    pub height: u32,
    /// Edge or corner of the anchor rectangle used for placement.
    pub anchor_edge: PopupAnchor,
    /// Popup edge or corner pulled toward the anchor.
    pub gravity: PopupGravity,
    /// Horizontal positioner offset in logical pixels.
    pub offset_x: i32,
    /// Vertical positioner offset in logical pixels.
    pub offset_y: i32,
    /// Compositor adjustments allowed when the popup would be constrained.
    pub constraints: PopupConstraints,
    /// Requests an explicit popup grab from the latest input serial.
    pub grab_focus: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PopupAnchor {
    None,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    #[default]
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PopupGravity {
    None,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PopupConstraints {
    pub slide_x: bool,
    pub slide_y: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    pub resize_x: bool,
    pub resize_y: bool,
}

impl Default for PopupConstraints {
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

impl Default for PopupConfig {
    fn default() -> Self {
        Self {
            anchor: InputRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            width: 1,
            height: 1,
            anchor_edge: PopupAnchor::default(),
            gravity: PopupGravity::default(),
            offset_x: 0,
            offset_y: 0,
            constraints: PopupConstraints::default(),
            grab_focus: false,
        }
    }
}

pub(crate) fn popup_anchor(anchor: PopupAnchor) -> xdg_positioner::Anchor {
    match anchor {
        PopupAnchor::None => xdg_positioner::Anchor::None,
        PopupAnchor::Top => xdg_positioner::Anchor::Top,
        PopupAnchor::Bottom => xdg_positioner::Anchor::Bottom,
        PopupAnchor::Left => xdg_positioner::Anchor::Left,
        PopupAnchor::Right => xdg_positioner::Anchor::Right,
        PopupAnchor::TopLeft => xdg_positioner::Anchor::TopLeft,
        PopupAnchor::TopRight => xdg_positioner::Anchor::TopRight,
        PopupAnchor::BottomLeft => xdg_positioner::Anchor::BottomLeft,
        PopupAnchor::BottomRight => xdg_positioner::Anchor::BottomRight,
    }
}

pub(crate) fn popup_gravity(gravity: PopupGravity) -> xdg_positioner::Gravity {
    match gravity {
        PopupGravity::None => xdg_positioner::Gravity::None,
        PopupGravity::Top => xdg_positioner::Gravity::Top,
        PopupGravity::Bottom => xdg_positioner::Gravity::Bottom,
        PopupGravity::Left => xdg_positioner::Gravity::Left,
        PopupGravity::Right => xdg_positioner::Gravity::Right,
        PopupGravity::TopLeft => xdg_positioner::Gravity::TopLeft,
        PopupGravity::TopRight => xdg_positioner::Gravity::TopRight,
        PopupGravity::BottomLeft => xdg_positioner::Gravity::BottomLeft,
        PopupGravity::BottomRight => xdg_positioner::Gravity::BottomRight,
    }
}

pub(crate) fn popup_constraints(
    constraints: PopupConstraints,
) -> xdg_positioner::ConstraintAdjustment {
    let mut value = xdg_positioner::ConstraintAdjustment::empty();
    if constraints.slide_x {
        value |= xdg_positioner::ConstraintAdjustment::SlideX;
    }
    if constraints.slide_y {
        value |= xdg_positioner::ConstraintAdjustment::SlideY;
    }
    if constraints.flip_x {
        value |= xdg_positioner::ConstraintAdjustment::FlipX;
    }
    if constraints.flip_y {
        value |= xdg_positioner::ConstraintAdjustment::FlipY;
    }
    if constraints.resize_x {
        value |= xdg_positioner::ConstraintAdjustment::ResizeX;
    }
    if constraints.resize_y {
        value |= xdg_positioner::ConstraintAdjustment::ResizeY;
    }
    value
}

/// One window on the compositor, as `ext-foreign-toplevel-list-v1` describes it.
///
/// Deliberately thin. The protocol reports what a window *is* — a title, an
/// application, a stable name — and nothing about where it is or what it is
/// doing, because that is the compositor's business and not a client's. An
/// overview or a task switcher wants exactly this list, plus a capture of each,
/// and no more.
/// One workspace, as `ext-workspace-v1` describes it.
///
/// Compositor-neutral by construction: nothing here is Hyprland's or sway's
/// vocabulary, because the protocol is what both of them speak.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceInfo {
    /// The field to act on, and the one `activate` takes.
    ///
    /// Unique, and lives exactly as long as the workspace does. Not the name --
    /// names are for people, are not unique, and change.
    pub key: String,
    /// The compositor's own cross-session id, when it offers one.
    ///
    /// Optional in the protocol and empty on compositors that send none, so it
    /// is no use as a key. What it is good for is remembering a preference
    /// against a workspace between sessions, which is exactly what the protocol
    /// says it is for.
    pub id: String,
    /// What to show a person, which is often a number.
    pub name: String,
    /// Where it sits in the compositor's arrangement, however many dimensions
    /// that has. What they mean is the compositor's business; what a shell does
    /// with them is sort by them.
    pub coordinates: Vec<u32>,
    /// The output whose group it belongs to, so a per-screen bar can show its
    /// own workspaces rather than all of them.
    pub output: String,
    pub active: bool,
    /// The workspace is asking for attention.
    pub urgent: bool,
    /// The compositor would rather it were not listed.
    pub hidden: bool,
    /// Whether `activate` will do anything. A compositor may list a workspace
    /// it will not switch to, and a bar that offers the click anyway is a bar
    /// with a dead button on it.
    pub activatable: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToplevelInfo {
    /// Stable for the life of the window, and unique on this compositor.
    ///
    /// The one field to key on. Titles change while you read them and two
    /// windows of the same application share an app id.
    pub identifier: String,
    /// What the window calls itself, which is usually what to show a person.
    pub title: String,
    /// Which application it belongs to, matching a desktop entry's id where the
    /// application sets it — which is how an overview finds an icon.
    pub app_id: String,
}
