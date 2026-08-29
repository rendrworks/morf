//! Wayland layer surfaces, fractional scale, and compositor frame callbacks.

use std::collections::VecDeque;
use std::error::Error as StdError;
use std::fmt;
use std::num::NonZeroU32;
use std::ptr::NonNull;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    WaylandWindowHandle, WindowHandle,
};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
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
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            namespace: "mold".to_owned(),
            height: 32,
            exclusive_zone: 32,
        }
    }
}

/// Event produced by the layer-surface connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerEvent {
    /// The compositor selected a logical surface size.
    Configure { width: u32, height: u32 },
    /// The preferred scale changed in protocol-native 120ths.
    Scale(u32),
    /// The compositor permits the next animation and paint tick.
    Frame { time_ms: u32 },
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
        let surface = compositor.create_surface(&qh);
        surface.set_buffer_scale(1);
        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Top,
            Some(config.namespace),
            None,
        );
        layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, config.height);
        layer.set_exclusive_zone(config.exclusive_zone);

        let fractional_manager = globals
            .bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let viewporter = globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok();
        let fractional_scale = fractional_manager
            .as_ref()
            .map(|manager| manager.get_fractional_scale(layer.wl_surface(), &qh, ()));
        let viewport = viewporter
            .as_ref()
            .map(|manager| manager.get_viewport(layer.wl_surface(), &qh, ()));
        layer.commit();

        let state = LayerState {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            layer,
            _fractional_manager: fractional_manager,
            fractional_scale,
            _viewporter: viewporter,
            viewport,
            width: 1,
            height: config.height.max(1),
            scale_120: 120,
            events: VecDeque::new(),
        };
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

    /// Removes the next queued surface event.
    pub fn next_event(&mut self) -> Option<LayerEvent> {
        self.state.events.pop_front()
    }

    /// Requests a compositor callback for the next frame.
    pub fn request_frame(&self) {
        let qh = self.queue.handle();
        let surface = self.state.layer.wl_surface();
        surface.frame(&qh, FrameCallbackData(surface.clone()));
    }

    /// Commits pending surface state without attaching a buffer.
    pub fn commit(&self) {
        self.state.layer.commit();
    }

    /// Returns the underlying surface used to construct a GPU presentation target.
    pub fn surface(&self) -> &wl_surface::WlSurface {
        self.state.layer.wl_surface()
    }

    /// Returns a clone of the connection backend for a raw display handle.
    pub fn backend(&self) -> wayland_backend::client::Backend {
        self.connection.backend()
    }

    /// Returns an owned raw-window target suitable for wgpu surface creation.
    pub fn window_target(&self) -> WaylandWindowTarget {
        WaylandWindowTarget {
            backend: self.connection.backend(),
            surface: self.state.layer.wl_surface().clone(),
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
    outputs: OutputState,
    layer: LayerSurface,
    _fractional_manager: Option<WpFractionalScaleManagerV1>,
    fractional_scale: Option<WpFractionalScaleV1>,
    _viewporter: Option<WpViewporter>,
    viewport: Option<WpViewport>,
    width: u32,
    height: u32,
    scale_120: u32,
    events: VecDeque<LayerEvent>,
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
        if surface == self.layer.wl_surface() {
            self.events.push_back(LayerEvent::Frame { time_ms: time });
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
    }

    fn update_output(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
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

    registry_handlers![OutputState];
}

delegate_registry!(LayerState);
smithay_client_toolkit::delegate_dispatch2!(LayerState);
wayland_client::delegate_noop!(LayerState: ignore WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(LayerState: ignore WpViewporter);
wayland_client::delegate_noop!(LayerState: ignore WpViewport);

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
