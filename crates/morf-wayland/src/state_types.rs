use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    WaylandWindowHandle, WindowHandle,
};
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::data_device_manager::DataDeviceManagerState;
use smithay_client_toolkit::data_device_manager::data_device::DataDevice;
use smithay_client_toolkit::data_device_manager::data_source::CopyPasteSource;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::session_lock::{SessionLock, SessionLockState, SessionLockSurface};
use smithay_client_toolkit::shell::wlr_layer::LayerShell;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::popup::Popup;
use smithay_client_toolkit::shell::xdg::window::Window;
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::shm::slot::{Buffer as ShmBuffer, SlotPool};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, mpsc};
use std::time::Instant;
use wayland_client::Proxy;
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch,
};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1;
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::ExtIdleNotificationV1, ext_idle_notifier_v1::ExtIdleNotifierV1,
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
    ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
    ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::WpFractionalScaleV1,
};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3, zwp_text_input_v3::ZwpTextInputV3,
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2, zwp_input_method_v2::ZwpInputMethodV2,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::output_power_management::v1::client::{
    zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1, zwlr_output_power_v1::ZwlrOutputPowerV1,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use crate::{client_surface::*, surface_types::*, types::*};

/// Owned Wayland display and surface handles for graphics APIs.
#[derive(Clone, Debug)]
pub struct WaylandWindowTarget {
    pub(crate) backend: wayland_backend::client::Backend,
    pub(crate) surface: wl_surface::WlSurface,
}

impl HasDisplayHandle for WaylandWindowTarget {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.backend.display_handle()
    }
}

impl HasWindowHandle for WaylandWindowTarget {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let pointer =
            NonNull::new(self.surface.id().as_ptr().cast()).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// One live wlr-layer-shell surface and the per-surface state the compositor
/// configures independently of every other layer surface this client owns.
pub(crate) struct LayerRecord {
    pub(crate) surface: ShellSurface,
    pub(crate) fractional_scale: Option<WpFractionalScaleV1>,
    pub(crate) viewport: Option<WpViewport>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_120: u32,
    /// Whether this surface should map itself with a blank buffer once the
    /// compositor has configured it.
    pub(crate) wants_blank: bool,
    /// Whether a configure has been acknowledged, which the protocol requires
    /// before any buffer may be attached.
    pub(crate) configured: bool,
    /// This surface's background-effect object, created the first time it asks
    /// for a blurred backdrop. Kept because destroying it clears the region.
    pub(crate) backdrop: Option<ExtBackgroundEffectSurfaceV1>,
    /// Backing store for a surface mapped with a blank buffer.
    ///
    /// A reserver has no renderer, but a layer surface that never attaches a
    /// buffer stays unmapped, and a compositor computes an output's usable area
    /// only from the layer surfaces it actually arranges. Holding the pool and
    /// the buffer here keeps the mapping alive for as long as the surface is.
    pub(crate) blank: Option<(SlotPool, ShmBuffer)>,
}

impl Drop for LayerRecord {
    fn drop(&mut self) {
        if let Some(scale) = self.fractional_scale.take() {
            scale.destroy();
        }
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
    }
}

pub(crate) struct LayerState {
    pub(crate) registry: RegistryState,
    pub(crate) compositor: CompositorState,
    pub(crate) outputs: OutputState,
    pub(crate) seats: SeatState,
    pub(crate) xdg_shell: XdgShell,
    /// The layer shell, when the compositor offers one.
    ///
    /// Optional because `wlr-layer-shell` is an extension and kiosk
    /// compositors omit it; see `ShellSurface` for what happens instead.
    pub(crate) layer_shell: Option<LayerShell>,
    pub(crate) layers: HashMap<u64, LayerRecord>,
    pub(crate) popups: HashMap<u64, Popup>,
    /// Reposition tokens sent to, and echoed back by, each live popup.
    pub(crate) popup_repositions: HashMap<u64, PopupReposition>,
    pub(crate) floatings: HashMap<u64, Window>,
    pub(crate) floating_sizes: HashMap<u64, (u32, u32)>,
    pub(crate) fractional_manager: Option<WpFractionalScaleManagerV1>,
    pub(crate) viewporter: Option<WpViewporter>,
    pub(crate) events: VecDeque<LayerEvent>,
    pub(crate) pointer: Option<wl_pointer::WlPointer>,
    pub(crate) pointer_seat: Option<wl_seat::WlSeat>,
    pub(crate) keyboard: Option<wl_keyboard::WlKeyboard>,
    pub(crate) touch: Option<wl_touch::WlTouch>,
    pub(crate) touch_points: HashMap<i32, ((f64, f64), SurfaceRole)>,
    pub(crate) keyboard_surface: Option<SurfaceRole>,
    pub(crate) idle_notifier: Option<ExtIdleNotifierV1>,
    pub(crate) idle_inhibit_manager: Option<ZwpIdleInhibitManagerV1>,
    /// The live inhibitor, if the shell is currently holding the session awake.
    ///
    /// Its existence *is* the inhibition — the protocol has no "off", only a
    /// destroy — so this is `Some` exactly while the session is being held.
    pub(crate) idle_inhibitor: Option<ZwpIdleInhibitorV1>,
    pub(crate) idle_notifications: Vec<ExtIdleNotificationV1>,
    pub(crate) idle_timeouts: Vec<u32>,
    pub(crate) data_device_manager: Option<DataDeviceManagerState>,
    pub(crate) data_devices: Vec<DataDevice>,
    pub(crate) clipboard_source: Option<CopyPasteSource>,
    pub(crate) clipboard_text: String,
    pub(crate) clipboard_tx: mpsc::Sender<Option<String>>,
    pub(crate) clipboard_rx: mpsc::Receiver<Option<String>>,
    pub(crate) clipboard_reads: Arc<AtomicUsize>,
    pub(crate) clipboard_writes: Arc<AtomicUsize>,
    pub(crate) latest_input_serial: Option<u32>,
    pub(crate) virtual_keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    pub(crate) virtual_keyboard: Option<ZwpVirtualKeyboardV1>,
    pub(crate) virtual_keyboard_keymap: Option<String>,
    pub(crate) virtual_keyboard_keymap_file: Option<File>,
    pub(crate) virtual_keyboard_clock: Instant,
    pub(crate) input_method_manager: Option<ZwpInputMethodManagerV2>,
    pub(crate) input_method: Option<ZwpInputMethodV2>,
    pub(crate) input_method_pending: InputMethodState,
    pub(crate) input_method_state: InputMethodState,
    pub(crate) text_input_manager: Option<ZwpTextInputManagerV3>,
    pub(crate) text_input: Option<ZwpTextInputV3>,
    pub(crate) text_input_requested: bool,
    pub(crate) text_input_pending: TextInputState,
    pub(crate) output_power_manager: Option<ZwlrOutputPowerManagerV1>,
    pub(crate) output_power: Vec<OutputPowerControl>,
    pub(crate) output_power_target: Option<wl_output::WlOutput>,
    pub(crate) output_power_mode: Option<OutputPowerMode>,
    pub(crate) shm: Option<Shm>,
    pub(crate) screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    /// `ext-background-effect-v1`, when the compositor offers it.
    ///
    /// The blur it asks for happens entirely on the compositor's side: it holds
    /// every window's buffer and is the only thing that can see what is behind
    /// this surface. A client never receives those pixels — it names a region
    /// and paints over the result with alpha.
    pub(crate) background_effect: Option<ExtBackgroundEffectManagerV1>,
    /// Whether the compositor currently advertises the blur capability.
    ///
    /// Sent when the manager is bound and again whenever it changes, so a
    /// compositor may withdraw it at run time — at which point it stops
    /// applying blur even to regions already set.
    pub(crate) blur_capable: bool,
    pub(crate) screencopies: Vec<PendingScreencopy>,
    pub(crate) screens: Vec<ScreenInfo>,
    /// `ext-foreign-toplevel-list-v1`, when the compositor offers it.
    pub(crate) toplevel_list: Option<ExtForeignToplevelListV1>,
    /// Every window the compositor has told us about, keyed by its handle.
    ///
    /// Held as a map because the protocol describes a window over several
    /// events and finishes with `done`: a handle arrives bare, then its title,
    /// app id and identifier follow, and only after `done` is it worth showing
    /// anybody.
    pub(crate) toplevels: HashMap<ObjectId, ToplevelInfo>,
    /// Whether the list changed since a caller last looked.
    pub(crate) toplevels_changed: bool,
    /// The handle behind each window, kept so a capture can name one.
    ///
    /// Separate from the descriptions because a configuration is given strings
    /// and hands one back: it never sees a protocol object, and the engine has
    /// to find its way from an identifier to the handle the compositor knows.
    pub(crate) toplevel_handles: HashMap<String, ExtForeignToplevelHandleV1>,
    /// `ext-image-copy-capture-v1` and the two source factories, when offered.
    ///
    /// The replacement for `wlr-screencopy`, and the reason to want it: that one
    /// captures outputs and only outputs, so a thumbnail of a *window* could not
    /// be had at all — cropping an output gives whatever is on top at that
    /// rectangle, not the window.
    pub(crate) capture_manager: Option<ExtImageCopyCaptureManagerV1>,
    pub(crate) output_source_manager: Option<ExtOutputImageCaptureSourceManagerV1>,
    pub(crate) toplevel_source_manager: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    /// Captures in flight on the newer protocol.
    pub(crate) captures: Vec<PendingCapture>,
    pub(crate) session_locks: SessionLockState,
    pub(crate) session_lock: Option<SessionLock>,
    pub(crate) lock_surfaces: Vec<LockSurface>,
}

pub(crate) struct OutputPowerControl {
    pub(crate) output: wl_output::WlOutput,
    pub(crate) control: ZwlrOutputPowerV1,
}

pub(crate) struct PendingScreencopy {
    pub(crate) request_id: u64,
    pub(crate) frame: ZwlrScreencopyFrameV1,
    pub(crate) offer: Option<(wl_shm::Format, u32, u32, u32)>,
    pub(crate) pool: Option<SlotPool>,
    pub(crate) buffer: Option<ShmBuffer>,
    pub(crate) format: Option<ScreencopyFormat>,
    pub(crate) y_invert: bool,
}

/// One capture in flight on `ext-image-copy-capture-v1`.
///
/// More states than the older protocol needed, because this one negotiates
/// before it copies: the session reports the size and formats it can produce,
/// and only once that is `done` is there anything to allocate a buffer against.
/// A frame is then created, given the buffer, and told to capture.
pub(crate) struct PendingCapture {
    pub(crate) request_id: u64,
    pub(crate) session: ExtImageCopyCaptureSessionV1,
    pub(crate) frame: Option<ExtImageCopyCaptureFrameV1>,
    /// Size the session says it will produce, from `buffer_size`.
    pub(crate) size: Option<(u32, u32)>,
    /// The first shared-memory format offered that this engine can carry.
    pub(crate) format: Option<wl_shm::Format>,
    pub(crate) pool: Option<SlotPool>,
    pub(crate) buffer: Option<ShmBuffer>,
    /// Whether the frame has been created and told to capture.
    pub(crate) started: bool,
}

pub(crate) struct LockSurface {
    pub(crate) surface: SessionLockSurface,
    pub(crate) output: wl_output::WlOutput,
    pub(crate) size: (u32, u32),
    pub(crate) scale: u32,
}
