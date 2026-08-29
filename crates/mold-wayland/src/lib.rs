//! Wayland layer surfaces, fractional scale, and compositor frame callbacks.

use std::collections::VecDeque;
use std::error::Error as StdError;
use std::fmt;
use std::num::NonZeroU32;
use std::ptr::NonNull;
use std::time::Duration;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    WaylandWindowHandle, WindowHandle,
};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
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
    wl_keyboard, wl_output, wl_pointer, wl_region, wl_seat, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols::xdg::shell::client::xdg_positioner;

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
            height: config.height.max(1),
            scale_120: 120,
            events: VecDeque::new(),
            pointer: None,
            keyboard: None,
            screens: Vec::new(),
        };
        let mut queue = queue;
        queue
            .roundtrip(&mut state)
            .map_err(|error| WaylandError(format!("could not read Wayland outputs: {error}")))?;
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

    /// Removes the next queued surface event.
    pub fn next_event(&mut self) -> Option<LayerEvent> {
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
    screens: Vec<ScreenInfo>,
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

impl OutputHandler for LayerState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.refresh_screens();
    }

    fn update_output(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.refresh_screens();
    }

    fn output_destroyed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.refresh_screens();
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
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
    ) {
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
    }

    fn remove_seat(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
    ) {
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
                PointerEventKind::Press { button, .. } => {
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
        _serial: u32,
        event: KeyEvent,
    ) {
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
        _serial: u32,
        modifiers: Modifiers,
        _raw: RawModifiers,
        _layout: u32,
    ) {
        self.events.push_back(LayerEvent::Modifiers {
            control: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            logo: modifiers.logo,
        });
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
wayland_client::delegate_noop!(LayerState: ignore WpViewport);
wayland_client::delegate_noop!(LayerState: ignore wl_region::WlRegion);

fn physical_size(logical: (u32, u32), scale_120: u32) -> (u32, u32) {
    let scale = scale_120.max(1) as u64;
    (
        ((logical.0 as u64 * scale).div_ceil(120)).max(1) as u32,
        ((logical.1 as u64 * scale).div_ceil(120)).max(1) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_size_rounds_fractional_scale_upward() {
        assert_eq!(physical_size((101, 31), 150), (127, 39));
    }
}
