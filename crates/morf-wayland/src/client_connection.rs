use crate::client_layer::PRIMARY_LAYER;
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::data_device_manager::DataDeviceManagerState;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::session_lock::SessionLockState;
use smithay_client_toolkit::shell::wlr_layer::LayerShell;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shm::Shm;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, mpsc};
use std::time::Instant;
use wayland_client::Connection;
use wayland_client::globals::registry_queue_init;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1;
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1;
use wayland_protocols::ext::workspace::v1::client::ext_workspace_manager_v1::ExtWorkspaceManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1;
use wayland_protocols_wlr::output_power_management::v1::client::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1;
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::{helpers::*, state_types::*, surface_types::*, types::*};

impl LayerClient {
    /// Connects to the current Wayland compositor and creates a top layer bar.
    pub fn connect(config: BarConfig) -> Result<Self, WaylandError> {
        Self::connect_inner(Some(config))
    }

    /// Connects without creating a visible surface for exclusive session locking.
    pub fn connect_lock() -> Result<Self, WaylandError> {
        Self::connect_inner(None)
    }

    /// Connects only to read what the compositor is offering.
    ///
    /// No surface. Asking the question through an ordinary connection used to
    /// mean creating one from a default configuration, which put a surface on
    /// screen that nothing had asked for: it reserved thirty-two pixels of
    /// everyone else's space and, being `on_demand`, would take the keyboard
    /// from whatever had it if the pointer found it first. Both were gone again
    /// within a frame, which is not the same as never having happened.
    pub fn probe() -> Result<Self, WaylandError> {
        Self::connect_inner(None)
    }

    pub(crate) fn connect_inner(config: Option<BarConfig>) -> Result<Self, WaylandError> {
        let connection = Connection::connect_to_env()
            .map_err(|error| WaylandError(format!("could not connect to Wayland: {error}")))?;
        let (globals, queue) = registry_queue_init(&connection)
            .map_err(|error| WaylandError(format!("could not read Wayland globals: {error}")))?;
        let qh = queue.handle();
        let compositor = CompositorState::bind(&globals, &qh)
            .map_err(|error| WaylandError(format!("wl_compositor is unavailable: {error}")))?;
        // Not fatal when missing. A compositor without layer-shell is a
        // compositor morf can still draw on, as a fullscreen toplevel; see
        // `ShellSurface`. Refusing to connect would rule out every kiosk
        // compositor, greetd's included, over an extension that is optional
        // by design.
        let layer_shell = LayerShell::bind(&globals, &qh).ok();
        let xdg_shell = XdgShell::bind(&globals, &qh)
            .map_err(|error| WaylandError(format!("xdg shell is unavailable: {error}")))?;
        let fractional_manager = globals
            .bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let viewporter = globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok();
        let idle_notifier = globals.bind::<ExtIdleNotifierV1, _, _>(&qh, 1..=2, ()).ok();
        let idle_inhibit_manager = globals
            .bind::<ZwpIdleInhibitManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let shortcuts_inhibit_manager = globals
            .bind::<ZwpKeyboardShortcutsInhibitManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let workspace_manager = globals
            .bind::<ExtWorkspaceManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        // Version 3 where offered, for `set_fullscreen`; 1 is enough for
        // activate, close and the maximize/minimize pair.
        let toplevel_control_manager = globals
            .bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ())
            .ok();
        let data_device_manager = DataDeviceManagerState::bind(&globals, &qh).ok();
        let virtual_keyboard_manager = globals
            .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let input_method_manager = globals
            .bind::<ZwpInputMethodManagerV2, _, _>(&qh, 1..=1, ())
            .ok();
        let text_input_manager = globals
            .bind::<ZwpTextInputManagerV3, _, _>(&qh, 1..=2, ())
            .ok();
        let output_power_manager = globals
            .bind::<ZwlrOutputPowerManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let shm = Shm::bind(&globals, &qh).ok();
        // Blur behind a surface. Absent on compositors that do not implement it,
        // in which case a configuration asking for one simply does not get it —
        // it is a finish, not a function.
        let background_effect = globals
            .bind::<ExtBackgroundEffectManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        // Every window the compositor knows about, and it tells us as they come
        // and go. A task switcher or an overview needs this list before it can
        // ask for a capture of anything in it.
        let toplevel_list = globals
            .bind::<ExtForeignToplevelListV1, _, _>(&qh, 1..=1, ())
            .ok();
        // The newer capture protocol, and the two things that name what to
        // capture. Bound separately because a compositor may offer the copy
        // machinery and only one kind of source.
        let capture_manager = globals
            .bind::<ExtImageCopyCaptureManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let output_source_manager = globals
            .bind::<ExtOutputImageCaptureSourceManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let toplevel_source_manager = globals
            .bind::<ExtForeignToplevelImageCaptureSourceManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let screencopy_manager = globals
            .bind::<ZwlrScreencopyManagerV1, _, _>(&qh, 1..=3, ())
            .ok();
        // Version 2 is where `create_immed` arrived, and nothing newer is
        // needed: the capture session, not this global, says which formats
        // a capture can use.
        let linux_dmabuf = globals.bind::<ZwpLinuxDmabufV1, _, _>(&qh, 2..=5, ()).ok();
        let session_locks = SessionLockState::new(&globals, &qh);
        let (clipboard_tx, clipboard_rx) = mpsc::channel();
        let mut state = LayerState {
            registry: RegistryState::new(&globals),
            compositor,
            outputs: OutputState::new(&globals, &qh),
            seats: SeatState::new(&globals, &qh),
            xdg_shell,
            layer_shell,
            layers: HashMap::new(),
            popups: HashMap::new(),
            popup_repositions: HashMap::new(),
            floatings: HashMap::new(),
            floating_sizes: HashMap::new(),
            fractional_manager,
            viewporter,
            events: VecDeque::new(),
            pointer: None,
            pointer_seat: None,
            keyboard: None,
            touch: None,
            touch_points: HashMap::new(),
            keyboard_surface: None,
            idle_notifier,
            idle_inhibit_manager,
            idle_inhibitor: None,
            shortcuts_inhibit_manager,
            shortcuts_inhibitor: None,
            toplevel_control_manager,
            toplevel_controls: HashMap::new(),
            toplevel_control_handles: HashMap::new(),
            aux_scales: HashMap::new(),
            workspace_manager,
            workspaces: HashMap::new(),
            workspace_handles: HashMap::new(),
            workspace_groups: HashMap::new(),
            workspace_group_handles: HashMap::new(),
            workspace_group_outputs: HashMap::new(),
            workspaces_changed: false,
            idle_notifications: Vec::new(),
            idle_timeouts: Vec::new(),
            data_device_manager,
            data_devices: Vec::new(),
            clipboard_source: None,
            clipboard_text: String::new(),
            clipboard_tx,
            clipboard_rx,
            clipboard_reads: Arc::new(AtomicUsize::new(0)),
            clipboard_writes: Arc::new(AtomicUsize::new(0)),
            latest_input_serial: None,
            virtual_keyboard_manager,
            virtual_keyboard: None,
            virtual_keyboard_keymap: default_keymap(),
            virtual_keyboard_keymap_file: None,
            virtual_keyboard_clock: Instant::now(),
            input_method_manager,
            input_method: None,
            input_method_pending: InputMethodState::default(),
            input_method_state: InputMethodState::default(),
            text_input_manager,
            text_input: None,
            text_input_requested: false,
            text_input_pending: TextInputState::default(),
            output_power_manager,
            output_power: Vec::new(),
            output_power_target: None,
            output_power_mode: None,
            shm,
            screencopy_manager,
            toplevel_list,
            toplevels: HashMap::new(),
            toplevels_changed: false,
            toplevel_handles: HashMap::new(),
            capture_manager,
            output_source_manager,
            toplevel_source_manager,
            captures: Vec::new(),
            linux_dmabuf,
            background_effect,
            // Assumed absent until the manager says otherwise: the capability
            // arrives as an event, so anything sent before it would be a guess.
            blur_capable: false,
            screencopies: Vec::new(),
            screens: Vec::new(),
            session_locks,
            session_lock: None,
            lock_surfaces: Vec::new(),
        };
        let mut queue = queue;
        queue
            .roundtrip(&mut state)
            .map_err(|error| WaylandError(format!("could not read Wayland outputs: {error}")))?;
        state.refresh_data_devices(&qh);
        let mut client = Self {
            connection,
            queue,
            state,
        };
        if let Some(config) = config {
            client.state.output_power_target = client.layer_output(config.output.as_deref())?;
            client.open_layer(PRIMARY_LAYER, config)?;
        }
        Ok(client)
    }
}
