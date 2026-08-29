//! Wayland layer surfaces, fractional scale, and compositor frame callbacks.

use std::collections::{HashMap, VecDeque};
use std::error::Error as StdError;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::os::fd::AsFd;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    WaylandWindowHandle, WindowHandle,
};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::{MemfdFlags, memfd_create};
use rustix::time::Timespec;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::{
    CopyPasteSource, DataSourceHandler,
};
use smithay_client_toolkit::data_device_manager::{DataDeviceManagerState, WritePipe};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keymap, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::touch::TouchHandler;
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::session_lock::{
    SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
    SessionLockSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::xdg::XdgPositioner;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::popup::{Popup, PopupConfigure, PopupHandler};
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{
    wl_data_device, wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_region, wl_seat,
    wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols::xdg::shell::client::xdg_positioner;
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::output_power_management::v1::client::{
    zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
    zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
};

/// Configuration for a top-anchored shell bar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BarConfig {
    /// Surface namespace exposed to the compositor.
    pub namespace: String,
    /// Requested logical height.
    pub height: u32,
    /// Layer-shell exclusive zone in logical pixels.
    pub exclusive_zone: i32,
    /// Compositor output name, or all outputs when unset.
    pub output: Option<String>,
}

/// Integer surface-local rectangle used to construct an input region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputRect {
    /// Left edge in logical pixels.
    pub x: i32,
    /// Top edge in logical pixels.
    pub y: i32,
    /// Positive width in logical pixels.
    pub width: i32,
    /// Positive height in logical pixels.
    pub height: i32,
}

/// Capability-derived compositor output description.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenInfo {
    /// Registry-global output identifier.
    pub id: u32,
    /// Compositor-provided stable output name when available.
    pub name: Option<String>,
    /// Logical top-left position.
    pub position: Option<(i32, i32)>,
    /// Logical output dimensions.
    pub size: Option<(i32, i32)>,
    /// Integer fallback scale advertised by wl_output.
    pub scale: i32,
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
}

/// Geometry and identity for an xdg toplevel surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FloatingConfig {
    /// Initial logical width.
    pub width: u32,
    /// Initial logical height.
    pub height: u32,
    /// Compositor-visible title.
    pub title: String,
    /// Desktop application identifier.
    pub app_id: String,
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

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            namespace: "mold".to_owned(),
            height: 32,
            exclusive_zone: 32,
            output: None,
        }
    }
}

/// Event produced by the layer-surface connection.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerEvent {
    /// The compositor selected a logical surface size.
    Configure { width: u32, height: u32 },
    /// The preferred scale changed in protocol-native 120ths.
    Scale(u32),
    /// The compositor permits the next animation and paint tick.
    Frame { time_ms: u32 },
    /// The pointer moved over or entered the surface.
    PointerMotion { x: f64, y: f64 },
    /// The pointer left the surface.
    PointerLeave,
    /// A pointer button changed state.
    PointerButton {
        button: u32,
        pressed: bool,
        x: f64,
        y: f64,
    },
    /// A touch contact began on the surface.
    TouchDown { id: i32, x: f64, y: f64 },
    /// A touch contact moved on the surface.
    TouchMotion { id: i32, x: f64, y: f64 },
    /// A touch contact ended on the surface.
    TouchUp { id: i32, x: f64, y: f64 },
    /// The compositor cancelled every active touch contact.
    TouchCancel,
    /// A keyboard key changed state.
    Key {
        keysym: u32,
        text: Option<String>,
        pressed: bool,
        repeat: bool,
    },
    /// Keyboard modifier state changed.
    Modifiers {
        control: bool,
        alt: bool,
        shift: bool,
        logo: bool,
    },
    /// A configured seat idle threshold changed state.
    Idle { timeout_ms: u32, idle: bool },
    /// A compositor output changed power state.
    OutputPower {
        output_id: u32,
        mode: OutputPowerMode,
    },
    /// The compositor clipboard selection changed.
    Clipboard { text: Option<String> },
    /// A focused text input committed a new input-method context.
    InputMethod(InputMethodState),
    /// The compositor output set changed.
    Screens(Vec<ScreenInfo>),
    /// The compositor positioned and sized the popup.
    PopupConfigure { width: u32, height: u32 },
    /// The compositor permits the next popup paint tick.
    PopupFrame { time_ms: u32 },
    /// The compositor dismissed the popup.
    PopupDone,
    /// The compositor configured the floating window.
    FloatingConfigure { width: u32, height: u32 },
    /// The compositor permits the next floating-window paint tick.
    FloatingFrame { time_ms: u32 },
    /// The compositor requested that the floating window close.
    FloatingClose,
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
    /// The compositor closed the layer surface.
    Closed,
}

/// Wayland connection or protocol setup failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaylandError(String);

impl fmt::Display for WaylandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for WaylandError {}

/// Live layer surface and its event queue.
pub struct LayerClient {
    connection: Connection,
    queue: EventQueue<LayerState>,
    state: LayerState,
}

impl LayerClient {
    /// Connects to the current Wayland compositor and creates a top layer bar.
    pub fn connect(config: BarConfig) -> Result<Self, WaylandError> {
        Self::connect_inner(Some(config))
    }

    /// Connects without creating a visible surface for exclusive session locking.
    pub fn connect_lock() -> Result<Self, WaylandError> {
        Self::connect_inner(None)
    }

    fn connect_inner(config: Option<BarConfig>) -> Result<Self, WaylandError> {
        let connection = Connection::connect_to_env()
            .map_err(|error| WaylandError(format!("could not connect to Wayland: {error}")))?;
        let (globals, queue) = registry_queue_init(&connection)
            .map_err(|error| WaylandError(format!("could not read Wayland globals: {error}")))?;
        let qh = queue.handle();
        let compositor = CompositorState::bind(&globals, &qh)
            .map_err(|error| WaylandError(format!("wl_compositor is unavailable: {error}")))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|error| WaylandError(format!("layer shell is unavailable: {error}")))?;
        let xdg_shell = XdgShell::bind(&globals, &qh)
            .map_err(|error| WaylandError(format!("xdg shell is unavailable: {error}")))?;
        let fractional_manager = globals
            .bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let viewporter = globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok();
        let idle_notifier = globals.bind::<ExtIdleNotifierV1, _, _>(&qh, 1..=2, ()).ok();
        let data_device_manager = DataDeviceManagerState::bind(&globals, &qh).ok();
        let virtual_keyboard_manager = globals
            .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let input_method_manager = globals
            .bind::<ZwpInputMethodManagerV2, _, _>(&qh, 1..=1, ())
            .ok();
        let output_power_manager = globals
            .bind::<ZwlrOutputPowerManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let session_locks = SessionLockState::new(&globals, &qh);
        let (clipboard_tx, clipboard_rx) = mpsc::channel();
        let mut state = LayerState {
            registry: RegistryState::new(&globals),
            compositor,
            outputs: OutputState::new(&globals, &qh),
            seats: SeatState::new(&globals, &qh),
            xdg_shell,
            layer: None,
            popup: None,
            floating: None,
            floating_size: (1, 1),
            _fractional_manager: fractional_manager,
            fractional_scale: None,
            _viewporter: viewporter,
            viewport: None,
            width: 1,
            height: config.as_ref().map_or(1, |config| config.height.max(1)),
            scale_120: 120,
            events: VecDeque::new(),
            pointer: None,
            keyboard: None,
            touch: None,
            touch_points: HashMap::new(),
            idle_notifier,
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
            output_power_manager,
            output_power: Vec::new(),
            output_power_target: None,
            output_power_mode: None,
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
        if let Some(config) = config {
            let output = match config.output.as_deref() {
                Some(name) => Some(
                    state
                        .outputs
                        .outputs()
                        .find(|output| {
                            state
                                .outputs
                                .info(output)
                                .and_then(|info| info.name)
                                .as_deref()
                                == Some(name)
                        })
                        .ok_or_else(|| {
                            WaylandError(format!("Wayland output `{name}` is unavailable"))
                        })?,
                ),
                None => None,
            };
            state.output_power_target = output.clone();
            let surface = state.compositor.create_surface(&qh);
            surface.set_buffer_scale(1);
            let layer = layer_shell.create_layer_surface(
                &qh,
                surface,
                Layer::Top,
                Some(config.namespace),
                output.as_ref(),
            );
            layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
            layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
            layer.set_size(0, config.height);
            layer.set_exclusive_zone(config.exclusive_zone);
            state.fractional_scale = state
                ._fractional_manager
                .as_ref()
                .map(|manager| manager.get_fractional_scale(layer.wl_surface(), &qh, ()));
            state.viewport = state
                ._viewporter
                .as_ref()
                .map(|manager| manager.get_viewport(layer.wl_surface(), &qh, ()));
            layer.commit();
            state.layer = Some(layer);
        }
        Ok(Self {
            connection,
            queue,
            state,
        })
    }

    /// Blocks until at least one Wayland event is dispatched.
    pub fn dispatch(&mut self) -> Result<(), WaylandError> {
        self.queue
            .blocking_dispatch(&mut self.state)
            .map_err(|error| WaylandError(format!("Wayland dispatch failed: {error}")))?;
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))?;
        Ok(())
    }

    /// Dispatches Wayland events or returns when the timeout expires.
    pub fn dispatch_timeout(&mut self, timeout: Duration) -> Result<bool, WaylandError> {
        if self
            .queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| WaylandError(format!("Wayland dispatch failed: {error}")))?
            > 0
        {
            return Ok(true);
        }
        self.queue
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))?;
        let Some(guard) = self.queue.prepare_read() else {
            return self
                .queue
                .dispatch_pending(&mut self.state)
                .map(|count| count > 0)
                .map_err(|error| WaylandError(format!("Wayland dispatch failed: {error}")));
        };
        let seconds = timeout.as_secs().min(i64::MAX as u64) as i64;
        let timeout = Timespec {
            tv_sec: seconds,
            tv_nsec: timeout.subsec_nanos() as i64,
        };
        let mut fds = [PollFd::new(&self.queue, PollFlags::IN)];
        let ready = poll(&mut fds, Some(&timeout))
            .map_err(|error| WaylandError(format!("Wayland poll failed: {error}")))?;
        if ready == 0 {
            drop(guard);
            return Ok(false);
        }
        guard
            .read()
            .map_err(|error| WaylandError(format!("Wayland read failed: {error}")))?;
        self.queue
            .dispatch_pending(&mut self.state)
            .map(|count| count > 0)
            .map_err(|error| WaylandError(format!("Wayland dispatch failed: {error}")))
    }

    /// Replaces seat idle thresholds and returns whether the compositor supports them.
    pub fn set_idle_timeouts(&mut self, timeouts: &[u32]) -> bool {
        self.state.idle_timeouts = timeouts.iter().copied().take(64).collect();
        self.state.idle_timeouts.sort_unstable();
        self.state.idle_timeouts.dedup();
        self.state.refresh_idle(&self.queue.handle());
        self.state.idle_notifier.is_some()
    }

    /// Requests a power state for the configured output, or every output for a lock client.
    pub fn set_output_power(&mut self, mode: OutputPowerMode) -> bool {
        if self.state.output_power_manager.is_none() {
            return false;
        }
        self.state.output_power_mode = Some(mode);
        let available = self.state.outputs.outputs().collect::<Vec<_>>();
        let outputs = match self.state.output_power_target.clone() {
            Some(output) => available
                .into_iter()
                .filter(|item| *item == output)
                .collect(),
            None => available,
        };
        let qh = self.queue.handle();
        for output in outputs {
            self.state.apply_output_power(&output, mode, &qh);
        }
        true
    }

    /// Publishes UTF-8 text to the clipboard after a compositor input serial is available.
    pub fn set_clipboard(&mut self, text: impl Into<String>) -> bool {
        let Some(manager) = &self.state.data_device_manager else {
            return false;
        };
        let Some(device) = self.state.data_devices.first() else {
            return false;
        };
        let Some(serial) = self.state.latest_input_serial else {
            return false;
        };
        let source = manager.create_copy_paste_source(
            &self.queue.handle(),
            ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"],
        );
        source.set_selection(device, serial);
        self.state.clipboard_text = text.into();
        self.state.clipboard_source = Some(source);
        true
    }

    /// Returns whether clipboard publication has a data device and a current input serial.
    pub fn can_set_clipboard(&self) -> bool {
        self.supports_clipboard() && self.state.latest_input_serial.is_some()
    }

    /// Returns whether the compositor exposes a clipboard data device.
    pub fn supports_clipboard(&self) -> bool {
        self.state.data_device_manager.is_some() && !self.state.data_devices.is_empty()
    }

    /// Sends one evdev keycode through the compositor virtual keyboard protocol.
    pub fn send_virtual_key(&mut self, keycode: u32, pressed: bool) -> bool {
        self.state.refresh_virtual_keyboard(&self.queue.handle());
        let Some(keyboard) = &self.state.virtual_keyboard else {
            return false;
        };
        if self.connection.flush().is_err() {
            return false;
        }
        let time = self
            .state
            .virtual_keyboard_clock
            .elapsed()
            .as_millis()
            .min(u32::MAX as u128) as u32;
        keyboard.key(time, keycode, u32::from(pressed));
        true
    }

    /// Sends virtual keyboard modifier masks and layout group.
    pub fn send_virtual_modifiers(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> bool {
        self.state.refresh_virtual_keyboard(&self.queue.handle());
        let Some(keyboard) = &self.state.virtual_keyboard else {
            return false;
        };
        if self.connection.flush().is_err() {
            return false;
        }
        keyboard.modifiers(depressed, latched, locked, group);
        true
    }

    /// Returns whether a virtual keyboard was created for the current seat.
    pub fn supports_virtual_keyboard(&self) -> bool {
        self.state.virtual_keyboard_manager.is_some()
            && self.state.seats.seats().next().is_some()
            && self.state.virtual_keyboard_keymap.is_some()
    }

    /// Claims the compositor input-method role for the current seat.
    pub fn enable_input_method(&mut self) -> bool {
        if self.state.input_method.is_some() {
            return true;
        }
        let Some(manager) = &self.state.input_method_manager else {
            return false;
        };
        let Some(seat) = self.state.seats.seats().next() else {
            return false;
        };
        self.state.input_method = Some(manager.get_input_method(&seat, &self.queue.handle(), ()));
        true
    }

    /// Returns whether the compositor exposes input-method-v2 for a seat.
    pub fn supports_input_method(&self) -> bool {
        self.state.input_method_manager.is_some() && self.state.seats.seats().next().is_some()
    }

    /// Commits UTF-8 text through the active input-method context.
    pub fn input_method_commit(&self, text: &str) -> bool {
        let Some(input_method) = &self.state.input_method else {
            return false;
        };
        input_method.commit_string(text.to_owned());
        input_method.commit(self.state.input_method_state.serial);
        true
    }

    /// Replaces the active preedit string and cursor range.
    pub fn input_method_preedit(&self, text: &str, begin: i32, end: i32) -> bool {
        let Some(input_method) = &self.state.input_method else {
            return false;
        };
        input_method.set_preedit_string(text.to_owned(), begin, end);
        input_method.commit(self.state.input_method_state.serial);
        true
    }

    /// Deletes byte ranges around the application cursor.
    pub fn input_method_delete(&self, before: u32, after: u32) -> bool {
        let Some(input_method) = &self.state.input_method else {
            return false;
        };
        input_method.delete_surrounding_text(before, after);
        input_method.commit(self.state.input_method_state.serial);
        true
    }

    /// Removes the next queued surface event.
    pub fn next_event(&mut self) -> Option<LayerEvent> {
        while let Ok(text) = self.state.clipboard_rx.try_recv() {
            self.state.events.push_back(LayerEvent::Clipboard { text });
        }
        self.state.events.pop_front()
    }

    /// Requests a compositor callback for the next frame.
    pub fn request_frame(&self) {
        let qh = self.queue.handle();
        let surface = self.state.layer().wl_surface();
        surface.frame(&qh, FrameCallbackData(surface.clone()));
    }

    /// Commits pending surface state without attaching a buffer.
    pub fn commit(&self) {
        self.state.layer().commit();
    }

    /// Returns the underlying surface used to construct a GPU presentation target.
    pub fn surface(&self) -> &wl_surface::WlSurface {
        self.state.layer().wl_surface()
    }

    /// Returns a clone of the connection backend for a raw display handle.
    pub fn backend(&self) -> wayland_backend::client::Backend {
        self.connection.backend()
    }

    /// Returns an owned raw-window target suitable for wgpu surface creation.
    pub fn window_target(&self) -> WaylandWindowTarget {
        WaylandWindowTarget {
            backend: self.connection.backend(),
            surface: self.state.layer().wl_surface().clone(),
        }
    }

    /// Returns the configured logical dimensions.
    pub fn logical_size(&self) -> (u32, u32) {
        (self.state.width, self.state.height)
    }

    /// Returns the preferred scale in 120ths.
    pub fn scale_120(&self) -> u32 {
        self.state.scale_120
    }

    /// Returns the physical buffer dimensions rounded upward.
    pub fn physical_size(&self) -> (u32, u32) {
        physical_size(self.logical_size(), self.scale_120())
    }

    /// Returns the latest compositor output snapshot.
    pub fn screens(&self) -> &[ScreenInfo] {
        &self.state.screens
    }

    /// Applies the default, empty, or explicitly rectangular input region.
    pub fn set_input_region(&self, rectangles: Option<&[InputRect]>) {
        let surface = self.state.layer().wl_surface();
        let Some(rectangles) = rectangles else {
            surface.set_input_region(None);
            return;
        };
        let qh = self.queue.handle();
        let region = self.state.compositor.wl_compositor().create_region(&qh, ());
        for rectangle in rectangles {
            if rectangle.width > 0 && rectangle.height > 0 {
                region.add(rectangle.x, rectangle.y, rectangle.width, rectangle.height);
            }
        }
        surface.set_input_region(Some(&region));
        region.destroy();
    }

    /// Creates an xdg popup anchored below a parent-surface rectangle.
    pub fn open_popup(&mut self, config: PopupConfig) -> Result<(), WaylandError> {
        self.close_popup();
        let qh = self.queue.handle();
        let positioner = XdgPositioner::new(&self.state.xdg_shell)
            .map_err(|error| WaylandError(format!("could not create popup positioner: {error}")))?;
        positioner.set_size(config.width.max(1) as i32, config.height.max(1) as i32);
        positioner.set_anchor_rect(
            config.anchor.x,
            config.anchor.y,
            config.anchor.width.max(1),
            config.anchor.height.max(1),
        );
        positioner.set_anchor(xdg_positioner::Anchor::BottomLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner.set_constraint_adjustment(
            xdg_positioner::ConstraintAdjustment::SlideX
                | xdg_positioner::ConstraintAdjustment::SlideY
                | xdg_positioner::ConstraintAdjustment::FlipX
                | xdg_positioner::ConstraintAdjustment::FlipY,
        );
        let surface = self.state.compositor.create_surface(&qh);
        surface.set_buffer_scale(1);
        let popup = Popup::from_surface(None, &positioner, &qh, surface, &self.state.xdg_shell)
            .map_err(|error| WaylandError(format!("could not create popup: {error}")))?;
        self.state.layer().get_popup(popup.xdg_popup());
        popup.wl_surface().commit();
        self.state.popup = Some(popup);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Destroys the current popup when present.
    pub fn close_popup(&mut self) {
        self.state.popup = None;
    }

    /// Returns the popup surface used to attach buffers.
    pub fn popup_surface(&self) -> Option<&wl_surface::WlSurface> {
        self.state.popup.as_ref().map(Popup::wl_surface)
    }

    /// Requests a compositor callback for the next popup frame.
    pub fn request_popup_frame(&self) {
        let Some(surface) = self.popup_surface() else {
            return;
        };
        let qh = self.queue.handle();
        surface.frame(&qh, FrameCallbackData(surface.clone()));
    }

    /// Returns an owned raw-window target for the current popup.
    pub fn popup_window_target(&self) -> Option<WaylandWindowTarget> {
        self.state.popup.as_ref().map(|popup| WaylandWindowTarget {
            backend: self.connection.backend(),
            surface: popup.wl_surface().clone(),
        })
    }

    /// Creates an undecorated xdg toplevel surface.
    pub fn open_floating(&mut self, config: FloatingConfig) -> Result<(), WaylandError> {
        self.close_floating();
        let qh = self.queue.handle();
        let surface = self.state.compositor.create_surface(&qh);
        surface.set_buffer_scale(1);
        let window = self
            .state
            .xdg_shell
            .create_window(surface, WindowDecorations::None, &qh);
        window.set_title(config.title);
        window.set_app_id(config.app_id);
        window.set_min_size(Some((1, 1)));
        self.state.floating_size = (config.width.max(1), config.height.max(1));
        window.wl_surface().commit();
        self.state.floating = Some(window);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Destroys the current floating window when present.
    pub fn close_floating(&mut self) {
        self.state.floating = None;
    }

    /// Returns the floating-window surface used to attach buffers.
    pub fn floating_surface(&self) -> Option<&wl_surface::WlSurface> {
        self.state.floating.as_ref().map(Window::wl_surface)
    }

    /// Requests a compositor callback for the next floating-window frame.
    pub fn request_floating_frame(&self) {
        let Some(surface) = self.floating_surface() else {
            return;
        };
        let qh = self.queue.handle();
        surface.frame(&qh, FrameCallbackData(surface.clone()));
    }

    /// Returns an owned raw-window target for the current floating window.
    pub fn floating_window_target(&self) -> Option<WaylandWindowTarget> {
        self.state
            .floating
            .as_ref()
            .map(|window| WaylandWindowTarget {
                backend: self.connection.backend(),
                surface: window.wl_surface().clone(),
            })
    }

    /// Requests exclusive compositor session ownership.
    pub fn begin_session_lock(&mut self) -> Result<(), WaylandError> {
        if self.state.session_lock.is_some() {
            return Err(WaylandError("session lock is already active".to_owned()));
        }
        let lock = self
            .state
            .session_locks
            .lock(&self.queue.handle())
            .map_err(|error| WaylandError(format!("session lock is unavailable: {error}")))?;
        self.state.session_lock = Some(lock);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Unlocks only after the compositor confirmed that the lock is active.
    pub fn unlock_session(&mut self) -> Result<(), WaylandError> {
        let lock = self
            .state
            .session_lock
            .take()
            .ok_or_else(|| WaylandError("session lock is not active".to_owned()))?;
        if !lock.is_locked() {
            self.state.session_lock = Some(lock);
            return Err(WaylandError(
                "session lock has not been confirmed by the compositor".to_owned(),
            ));
        }
        lock.unlock();
        self.state.lock_surfaces.clear();
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Returns one configured lock surface for rendering.
    pub fn lock_surface(&self, index: usize) -> Option<&wl_surface::WlSurface> {
        self.state
            .lock_surfaces
            .get(index)
            .map(|surface| surface.surface.wl_surface())
    }

    /// Returns one lock surface's configured logical size.
    pub fn lock_size(&self, index: usize) -> Option<(u32, u32)> {
        self.state
            .lock_surfaces
            .get(index)
            .map(|surface| surface.size)
    }

    /// Returns one lock surface's preferred integer scale in protocol 120ths.
    pub fn lock_scale_120(&self, index: usize) -> Option<u32> {
        self.state
            .lock_surfaces
            .get(index)
            .map(|surface| surface.scale.saturating_mul(120))
    }

    /// Returns one lock surface's physical buffer size.
    pub fn lock_physical_size(&self, index: usize) -> Option<(u32, u32)> {
        self.state.lock_surfaces.get(index).map(|surface| {
            (
                surface.size.0.saturating_mul(surface.scale),
                surface.size.1.saturating_mul(surface.scale),
            )
        })
    }

    /// Returns an owned raw-window target for one lock surface.
    pub fn lock_window_target(&self, index: usize) -> Option<WaylandWindowTarget> {
        self.lock_surface(index).map(|surface| WaylandWindowTarget {
            backend: self.connection.backend(),
            surface: surface.clone(),
        })
    }

    /// Requests a compositor frame callback for one lock surface.
    pub fn request_lock_frame(&self, index: usize) {
        let Some(surface) = self.lock_surface(index) else {
            return;
        };
        surface.frame(&self.queue.handle(), FrameCallbackData(surface.clone()));
    }

    /// Commits one lock surface without attaching a new buffer.
    pub fn commit_lock(&self, index: usize) {
        if let Some(surface) = self.lock_surface(index) {
            surface.commit();
        }
    }
}

/// Owned Wayland display and surface handles for graphics APIs.
#[derive(Clone, Debug)]
pub struct WaylandWindowTarget {
    backend: wayland_backend::client::Backend,
    surface: wl_surface::WlSurface,
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

struct LayerState {
    registry: RegistryState,
    compositor: CompositorState,
    outputs: OutputState,
    seats: SeatState,
    xdg_shell: XdgShell,
    layer: Option<LayerSurface>,
    popup: Option<Popup>,
    floating: Option<Window>,
    floating_size: (u32, u32),
    _fractional_manager: Option<WpFractionalScaleManagerV1>,
    fractional_scale: Option<WpFractionalScaleV1>,
    _viewporter: Option<WpViewporter>,
    viewport: Option<WpViewport>,
    width: u32,
    height: u32,
    scale_120: u32,
    events: VecDeque<LayerEvent>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    touch: Option<wl_touch::WlTouch>,
    touch_points: HashMap<i32, (f64, f64)>,
    idle_notifier: Option<ExtIdleNotifierV1>,
    idle_notifications: Vec<ExtIdleNotificationV1>,
    idle_timeouts: Vec<u32>,
    data_device_manager: Option<DataDeviceManagerState>,
    data_devices: Vec<DataDevice>,
    clipboard_source: Option<CopyPasteSource>,
    clipboard_text: String,
    clipboard_tx: mpsc::Sender<Option<String>>,
    clipboard_rx: mpsc::Receiver<Option<String>>,
    clipboard_reads: Arc<AtomicUsize>,
    clipboard_writes: Arc<AtomicUsize>,
    latest_input_serial: Option<u32>,
    virtual_keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    virtual_keyboard: Option<ZwpVirtualKeyboardV1>,
    virtual_keyboard_keymap: Option<String>,
    virtual_keyboard_keymap_file: Option<File>,
    virtual_keyboard_clock: Instant,
    input_method_manager: Option<ZwpInputMethodManagerV2>,
    input_method: Option<ZwpInputMethodV2>,
    input_method_pending: InputMethodState,
    input_method_state: InputMethodState,
    output_power_manager: Option<ZwlrOutputPowerManagerV1>,
    output_power: Vec<OutputPowerControl>,
    output_power_target: Option<wl_output::WlOutput>,
    output_power_mode: Option<OutputPowerMode>,
    screens: Vec<ScreenInfo>,
    session_locks: SessionLockState,
    session_lock: Option<SessionLock>,
    lock_surfaces: Vec<LockSurface>,
}

struct OutputPowerControl {
    output: wl_output::WlOutput,
    control: ZwlrOutputPowerV1,
}

struct LockSurface {
    surface: SessionLockSurface,
    output: wl_output::WlOutput,
    size: (u32, u32),
    scale: u32,
}

impl LayerState {
    fn refresh_virtual_keyboard(&mut self, qh: &QueueHandle<Self>) {
        if self.virtual_keyboard.is_some() {
            return;
        }
        let Some(manager) = &self.virtual_keyboard_manager else {
            return;
        };
        let Some(seat) = self.seats.seats().next() else {
            return;
        };
        let Some(keymap) = &self.virtual_keyboard_keymap else {
            return;
        };
        let keyboard = manager.create_virtual_keyboard(&seat, qh, ());
        match install_virtual_keymap(&keyboard, keymap) {
            Ok(file) => {
                self.virtual_keyboard_keymap_file = Some(file);
                self.virtual_keyboard = Some(keyboard);
            }
            Err(_) => keyboard.destroy(),
        }
    }

    fn refresh_data_devices(&mut self, qh: &QueueHandle<Self>) {
        let Some(manager) = &self.data_device_manager else {
            return;
        };
        for seat in self.seats.seats() {
            if self
                .data_devices
                .iter()
                .all(|device| device.data().seat() != &seat)
            {
                self.data_devices.push(manager.get_data_device(qh, &seat));
            }
        }
    }

    fn apply_output_power(
        &mut self,
        output: &wl_output::WlOutput,
        mode: OutputPowerMode,
        qh: &QueueHandle<Self>,
    ) {
        let Some(manager) = self.output_power_manager.clone() else {
            return;
        };
        let control = self
            .output_power
            .iter()
            .find(|control| control.output == *output)
            .map(|control| control.control.clone())
            .unwrap_or_else(|| {
                let control = manager.get_output_power(output, qh, output.clone());
                self.output_power.push(OutputPowerControl {
                    output: output.clone(),
                    control: control.clone(),
                });
                control
            });
        control.set_mode(match mode {
            OutputPowerMode::Off => zwlr_output_power_v1::Mode::Off,
            OutputPowerMode::On => zwlr_output_power_v1::Mode::On,
        });
    }

    fn refresh_idle(&mut self, qh: &QueueHandle<Self>) {
        for notification in self.idle_notifications.drain(..) {
            notification.destroy();
        }
        let Some(notifier) = &self.idle_notifier else {
            return;
        };
        let Some(seat) = self.seats.seats().next() else {
            return;
        };
        self.idle_notifications = self
            .idle_timeouts
            .iter()
            .map(|timeout| notifier.get_idle_notification(*timeout, &seat, qh, *timeout))
            .collect();
    }

    fn create_lock_surface(&mut self, output: wl_output::WlOutput, qh: &QueueHandle<Self>) {
        let Some(lock) = self.session_lock.clone().filter(SessionLock::is_locked) else {
            return;
        };
        if self
            .lock_surfaces
            .iter()
            .any(|surface| surface.output == output)
        {
            return;
        }
        let scale = self
            .outputs
            .info(&output)
            .map(|info| info.scale_factor.max(1) as u32)
            .unwrap_or(1);
        let surface = self.compositor.create_surface(qh);
        surface.set_buffer_scale(scale as i32);
        let surface = lock.create_lock_surface(surface, &output, qh);
        surface.wl_surface().commit();
        self.lock_surfaces.push(LockSurface {
            surface,
            output,
            size: (1, 1),
            scale,
        });
    }
}

impl CompositorHandler for LayerState {
    fn scale_factor_changed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if self.fractional_scale.is_none() {
            self.scale_120 = factor.max(1) as u32 * 120;
            self.events.push_back(LayerEvent::Scale(self.scale_120));
        }
    }

    fn transform_changed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        time: u32,
    ) {
        if self
            .layer
            .as_ref()
            .is_some_and(|layer| surface == layer.wl_surface())
        {
            self.events.push_back(LayerEvent::Frame { time_ms: time });
        } else if self
            .popup
            .as_ref()
            .is_some_and(|popup| surface == popup.wl_surface())
        {
            self.events
                .push_back(LayerEvent::PopupFrame { time_ms: time });
        } else if self
            .floating
            .as_ref()
            .is_some_and(|window| surface == window.wl_surface())
        {
            self.events
                .push_back(LayerEvent::FloatingFrame { time_ms: time });
        } else if let Some(index) = self
            .lock_surfaces
            .iter()
            .position(|lock| surface == lock.surface.wl_surface())
        {
            self.events.push_back(LayerEvent::SessionLockFrame {
                index,
                time_ms: time,
            });
        }
    }

    fn surface_enter(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for LayerState {
    fn closed(&mut self, _connection: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.events.push_back(LayerEvent::Closed);
    }

    fn configure(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.width = NonZeroU32::new(configure.new_size.0).map_or(self.width, NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(self.height, NonZeroU32::get);
        if let Some(viewport) = &self.viewport {
            viewport.set_destination(self.width as i32, self.height as i32);
        }
        self.events.push_back(LayerEvent::Configure {
            width: self.width,
            height: self.height,
        });
    }
}

impl SessionLockHandler for LayerState {
    fn locked(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        _session_lock: SessionLock,
    ) {
        self.lock_surfaces.clear();
        for output in self.outputs.outputs() {
            self.create_lock_surface(output, qh);
        }
        self.events.push_back(LayerEvent::SessionLocked);
    }

    fn finished(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _session_lock: SessionLock,
    ) {
        self.lock_surfaces.clear();
        self.session_lock = None;
        self.events.push_back(LayerEvent::SessionLockFinished);
    }

    fn configure(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self
            .lock_surfaces
            .iter()
            .position(|lock| lock.surface.wl_surface() == surface.wl_surface())
        else {
            return;
        };
        let size = (configure.new_size.0.max(1), configure.new_size.1.max(1));
        self.lock_surfaces[index].size = size;
        self.events.push_back(LayerEvent::SessionLockConfigure {
            index,
            width: size.0,
            height: size.1,
        });
    }
}

impl OutputHandler for LayerState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.refresh_screens();
        if self.output_power_target.is_none()
            && let Some(mode) = self.output_power_mode
        {
            self.apply_output_power(&output, mode, qh);
        }
        self.create_lock_surface(output, qh);
    }

    fn update_output(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.refresh_screens();
        let scale = self
            .outputs
            .info(&output)
            .map(|info| info.scale_factor.max(1) as u32)
            .unwrap_or(1);
        if let Some((index, surface)) = self
            .lock_surfaces
            .iter_mut()
            .enumerate()
            .find(|(_, surface)| surface.output == output)
            && surface.scale != scale
        {
            surface.scale = scale;
            surface.surface.wl_surface().set_buffer_scale(scale as i32);
            surface.surface.wl_surface().commit();
            self.events.push_back(LayerEvent::SessionLockConfigure {
                index,
                width: surface.size.0,
                height: surface.size.1,
            });
        }
    }

    fn output_destroyed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.refresh_screens();
        if let Some(index) = self
            .output_power
            .iter()
            .position(|control| control.output == output)
        {
            self.output_power.remove(index).control.destroy();
        }
        if let Some(index) = self
            .lock_surfaces
            .iter()
            .position(|surface| surface.output == output)
        {
            self.lock_surfaces.remove(index);
            self.events
                .push_back(LayerEvent::SessionLockSurfaceRemoved { index });
        }
    }
}

impl PopupHandler for LayerState {
    fn configure(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _popup: &Popup,
        config: PopupConfigure,
    ) {
        self.events.push_back(LayerEvent::PopupConfigure {
            width: config.width.max(1) as u32,
            height: config.height.max(1) as u32,
        });
    }

    fn done(&mut self, _connection: &Connection, _qh: &QueueHandle<Self>, _popup: &Popup) {
        self.popup = None;
        self.events.push_back(LayerEvent::PopupDone);
    }
}

impl WindowHandler for LayerState {
    fn request_close(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
    ) {
        self.floating = None;
        self.events.push_back(LayerEvent::FloatingClose);
    }

    fn configure(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let width = configure
            .new_size
            .0
            .map_or(self.floating_size.0, NonZeroU32::get);
        let height = configure
            .new_size
            .1
            .map_or(self.floating_size.1, NonZeroU32::get);
        self.floating_size = (width, height);
        self.events
            .push_back(LayerEvent::FloatingConfigure { width, height });
    }
}

impl SeatHandler for LayerState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
    ) {
        self.refresh_data_devices(qh);
        self.refresh_idle(qh);
    }

    fn new_capability(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seats.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seats.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Touch && self.touch.is_none() {
            self.touch = self.seats.get_touch(qh, &seat).ok();
        }
    }

    fn remove_capability(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
        }
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.keyboard.take()
        {
            keyboard.release();
        }
        if capability == Capability::Touch
            && let Some(touch) = self.touch.take()
        {
            touch.release();
            self.touch_points.clear();
        }
    }

    fn remove_seat(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
    ) {
        self.data_devices
            .retain(|device| device.data().seat() != &seat);
    }
}

impl PointerHandler for LayerState {
    fn pointer_frame(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if self
                .layer
                .as_ref()
                .is_none_or(|layer| &event.surface != layer.wl_surface())
            {
                continue;
            }
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.events.push_back(LayerEvent::PointerMotion { x, y })
                }
                PointerEventKind::Leave { .. } => {
                    self.events.push_back(LayerEvent::PointerLeave);
                }
                PointerEventKind::Press { button, serial, .. } => {
                    self.latest_input_serial = Some(serial);
                    self.events.push_back(LayerEvent::PointerButton {
                        button,
                        pressed: true,
                        x,
                        y,
                    });
                }
                PointerEventKind::Release { button, .. } => {
                    self.events.push_back(LayerEvent::PointerButton {
                        button,
                        pressed: false,
                        x,
                        y,
                    });
                }
                PointerEventKind::Axis { .. } => {}
            }
        }
    }
}

impl TouchHandler for LayerState {
    fn down(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        serial: u32,
        _time: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        if self
            .layer
            .as_ref()
            .is_none_or(|layer| surface != *layer.wl_surface())
        {
            return;
        }
        self.latest_input_serial = Some(serial);
        self.touch_points.insert(id, position);
        self.events.push_back(LayerEvent::TouchDown {
            id,
            x: position.0,
            y: position.1,
        });
    }

    fn up(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        id: i32,
    ) {
        if let Some((x, y)) = self.touch_points.remove(&id) {
            self.events.push_back(LayerEvent::TouchUp { id, x, y });
        }
    }

    fn motion(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        if let Some(point) = self.touch_points.get_mut(&id) {
            *point = position;
            self.events.push_back(LayerEvent::TouchMotion {
                id,
                x: position.0,
                y: position.1,
            });
        }
    }

    fn shape(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }

    fn cancel(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
    ) {
        self.touch_points.clear();
        self.events.push_back(LayerEvent::TouchCancel);
    }
}

impl KeyboardHandler for LayerState {
    fn enter(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.latest_input_serial = Some(serial);
        self.push_key(event, true, false);
    }

    fn repeat_key(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.push_key(event, true, true);
    }

    fn release_key(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.push_key(event, false, false);
    }

    fn update_modifiers(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        modifiers: Modifiers,
        _raw: RawModifiers,
        _layout: u32,
    ) {
        self.latest_input_serial = Some(serial);
        self.events.push_back(LayerEvent::Modifiers {
            control: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            logo: modifiers.logo,
        });
    }

    fn update_keymap(
        &mut self,
        connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        keymap: Keymap<'_>,
    ) {
        let keymap = keymap.as_string();
        self.virtual_keyboard_keymap = Some(keymap.clone());
        if let Some(keyboard) = &self.virtual_keyboard
            && let Ok(file) = install_virtual_keymap(keyboard, &keymap)
            && connection.flush().is_ok()
        {
            self.virtual_keyboard_keymap_file = Some(file);
        }
    }
}

impl DataDeviceHandler for LayerState {
    fn enter(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wl_data_device::WlDataDevice,
        _x: f64,
        _y: f64,
        _surface: &wl_surface::WlSurface,
    ) {
    }

    fn leave(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wl_data_device::WlDataDevice,
    ) {
    }

    fn motion(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wl_data_device::WlDataDevice,
        _x: f64,
        _y: f64,
    ) {
    }

    fn selection(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &wl_data_device::WlDataDevice,
    ) {
        let Some(offer) = self
            .data_devices
            .iter()
            .find(|device| device.inner() == data_device)
            .and_then(|device| device.data().selection_offer())
        else {
            self.events.push_back(LayerEvent::Clipboard { text: None });
            return;
        };
        let mime = offer.with_mime_types(|types| {
            ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
                .into_iter()
                .find(|preferred| types.iter().any(|mime| mime == preferred))
                .map(str::to_owned)
        });
        let Some(mime) = mime else {
            self.events.push_back(LayerEvent::Clipboard { text: None });
            return;
        };
        let Ok(pipe) = offer.receive(mime) else {
            self.events.push_back(LayerEvent::Clipboard { text: None });
            return;
        };
        if self
            .clipboard_reads
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
                (active < 8).then_some(active + 1)
            })
            .is_err()
        {
            return;
        }
        let tx = self.clipboard_tx.clone();
        let active = Arc::clone(&self.clipboard_reads);
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let text = pipe
                .take(1_048_577)
                .read_to_end(&mut bytes)
                .ok()
                .filter(|_| bytes.len() <= 1_048_576)
                .and_then(|_| String::from_utf8(bytes).ok());
            let _ = tx.send(text);
            active.fetch_sub(1, Ordering::Relaxed);
        });
    }

    fn drop_performed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wl_data_device::WlDataDevice,
    ) {
    }
}

impl DataOfferHandler for LayerState {
    fn source_actions(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        _actions: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        _actions: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }
}

impl DataSourceHandler for LayerState {
    fn accept_mime(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wl_data_source::WlDataSource,
        _mime: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        source: &wl_data_source::WlDataSource,
        _mime: String,
        mut pipe: WritePipe,
    ) {
        let Some(text) = self
            .clipboard_source
            .as_ref()
            .filter(|current| current.inner() == source)
            .map(|_| self.clipboard_text.clone())
        else {
            return;
        };
        if self
            .clipboard_writes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
                (active < 8).then_some(active + 1)
            })
            .is_err()
        {
            return;
        }
        let active = Arc::clone(&self.clipboard_writes);
        thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
            active.fetch_sub(1, Ordering::Relaxed);
        });
    }

    fn cancelled(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        source: &wl_data_source::WlDataSource,
    ) {
        if self
            .clipboard_source
            .as_ref()
            .is_some_and(|current| current.inner() == source)
        {
            self.clipboard_source = None;
        }
    }

    fn dnd_dropped(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wl_data_source::WlDataSource,
    ) {
    }

    fn dnd_finished(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wl_data_source::WlDataSource,
    ) {
    }

    fn action(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wl_data_source::WlDataSource,
        _action: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }
}

impl LayerState {
    fn layer(&self) -> &LayerSurface {
        self.layer
            .as_ref()
            .expect("layer surface is initialized before client use")
    }

    fn refresh_screens(&mut self) {
        let screens = self
            .outputs
            .outputs()
            .filter_map(|output| self.outputs.info(&output))
            .map(|info| ScreenInfo {
                id: info.id,
                name: info.name,
                position: info.logical_position,
                size: info.logical_size,
                scale: info.scale_factor,
            })
            .collect::<Vec<_>>();
        if screens != self.screens {
            self.screens = screens.clone();
            self.events.push_back(LayerEvent::Screens(screens));
        }
    }

    fn push_key(&mut self, event: KeyEvent, pressed: bool, repeat: bool) {
        self.events.push_back(LayerEvent::Key {
            keysym: event.keysym.raw(),
            text: event.utf8,
            pressed,
            repeat,
        });
    }
}

impl Dispatch<WpFractionalScaleV1, ()> for LayerState {
    fn event(
        state: &mut Self,
        _proxy: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.scale_120 = scale.max(1);
            state.events.push_back(LayerEvent::Scale(state.scale_120));
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, u32> for LayerState {
    fn event(
        state: &mut Self,
        _proxy: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        timeout_ms: &u32,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let idle = match event {
            ext_idle_notification_v1::Event::Idled => true,
            ext_idle_notification_v1::Event::Resumed => false,
            _ => return,
        };
        state.events.push_back(LayerEvent::Idle {
            timeout_ms: *timeout_ms,
            idle,
        });
    }
}

impl Dispatch<ZwlrOutputPowerV1, wl_output::WlOutput> for LayerState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputPowerV1,
        event: zwlr_output_power_v1::Event,
        output: &wl_output::WlOutput,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_power_v1::Event::Mode { mode } => {
                let mode = match mode {
                    wayland_client::WEnum::Value(zwlr_output_power_v1::Mode::Off) => {
                        OutputPowerMode::Off
                    }
                    wayland_client::WEnum::Value(zwlr_output_power_v1::Mode::On) => {
                        OutputPowerMode::On
                    }
                    _ => return,
                };
                let output_id = state.outputs.info(output).map(|info| info.id).unwrap_or(0);
                state
                    .events
                    .push_back(LayerEvent::OutputPower { output_id, mode });
            }
            zwlr_output_power_v1::Event::Failed => {
                if let Some(index) = state
                    .output_power
                    .iter()
                    .position(|control| control.control == *proxy)
                {
                    state.output_power.remove(index).control.destroy();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for LayerState {
    fn event(
        state: &mut Self,
        proxy: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => {
                state.input_method_pending = InputMethodState {
                    active: true,
                    serial: state.input_method_state.serial,
                    ..InputMethodState::default()
                };
            }
            zwp_input_method_v2::Event::Deactivate => {
                state.input_method_pending.active = false;
            }
            zwp_input_method_v2::Event::SurroundingText {
                text,
                cursor,
                anchor,
            } => {
                state.input_method_pending.surrounding_text = Some(text);
                state.input_method_pending.cursor = cursor;
                state.input_method_pending.anchor = anchor;
            }
            zwp_input_method_v2::Event::Done => {
                state.input_method_pending.serial = state.input_method_state.serial.wrapping_add(1);
                state.input_method_state = state.input_method_pending.clone();
                state
                    .events
                    .push_back(LayerEvent::InputMethod(state.input_method_state.clone()));
            }
            zwp_input_method_v2::Event::Unavailable => {
                if state.input_method.as_ref() == Some(proxy) {
                    state.input_method = None;
                }
                state.input_method_state.active = false;
                state
                    .events
                    .push_back(LayerEvent::InputMethod(state.input_method_state.clone()));
                proxy.destroy();
            }
            _ => {}
        }
    }
}

impl ProvidesRegistryState for LayerState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

delegate_registry!(LayerState);
smithay_client_toolkit::delegate_dispatch2!(LayerState);
wayland_client::delegate_noop!(LayerState: ignore WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(LayerState: ignore WpViewporter);
wayland_client::delegate_noop!(LayerState: ignore ExtIdleNotifierV1);
wayland_client::delegate_noop!(LayerState: ignore ZwlrOutputPowerManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(LayerState: ignore WpViewport);
wayland_client::delegate_noop!(LayerState: ignore wl_region::WlRegion);

fn physical_size(logical: (u32, u32), scale_120: u32) -> (u32, u32) {
    let scale = scale_120.max(1) as u64;
    (
        ((logical.0 as u64 * scale).div_ceil(120)).max(1) as u32,
        ((logical.1 as u64 * scale).div_ceil(120)).max(1) as u32,
    )
}

fn default_keymap() -> Option<String> {
    let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
    xkbcommon::xkb::Keymap::new_from_names(
        &context,
        "",
        "pc105",
        "us",
        "",
        None,
        xkbcommon::xkb::COMPILE_NO_FLAGS,
    )
    .map(|keymap| keymap.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1))
}

fn install_virtual_keymap(keyboard: &ZwpVirtualKeyboardV1, keymap: &str) -> std::io::Result<File> {
    let mut bytes = keymap.as_bytes().to_vec();
    if !bytes.ends_with(&[0]) {
        bytes.push(0);
    }
    let fd = memfd_create("mold-keymap", MemfdFlags::CLOEXEC)?;
    let mut file = File::from(fd);
    file.write_all(&bytes)?;
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    keyboard.keymap(1, file.as_fd(), bytes.len() as u32);
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_size_rounds_fractional_scale_upward() {
        assert_eq!(physical_size((101, 31), 150), (127, 39));
    }

    #[test]
    fn default_virtual_keymap_round_trips() {
        let keymap = default_keymap().unwrap();
        let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
        assert!(
            xkbcommon::xkb::Keymap::new_from_string(
                &context,
                keymap,
                xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1,
                xkbcommon::xkb::COMPILE_NO_FLAGS,
            )
            .is_some()
        );
    }
}
