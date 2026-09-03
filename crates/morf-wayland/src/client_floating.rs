use smithay_client_toolkit::compositor::FrameCallbackData;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::window::{Window, WindowDecorations};
use wayland_client::protocol::wl_surface;

use crate::{state_types::*, surface_types::*};

impl LayerClient {
    /// Creates an undecorated xdg toplevel surface.
    pub fn open_floating(
        &mut self,
        id: u64,
        parent: Option<u64>,
        config: FloatingConfig,
    ) -> Result<(), WaylandError> {
        self.close_floating(id);
        let qh = self.queue.handle();
        let surface = self.state.compositor.create_surface(&qh);
        surface.set_buffer_scale(1);
        let window = self
            .state
            .xdg_shell
            .create_window(surface, WindowDecorations::None, &qh);
        if let Some(parent) = parent {
            let parent = self
                .state
                .floatings
                .get(&parent)
                .ok_or_else(|| WaylandError("floating parent is not open".into()))?;
            window.set_parent(Some(parent));
        }
        window.set_title(config.title);
        window.set_app_id(config.app_id);
        window.set_min_size(Some((config.minimum_width, config.minimum_height)));
        if config.maximum_width.is_some() || config.maximum_height.is_some() {
            window.set_max_size(Some((
                config.maximum_width.unwrap_or_default(),
                config.maximum_height.unwrap_or_default(),
            )));
        }
        if config.maximized {
            window.set_maximized();
        }
        if config.fullscreen {
            window.set_fullscreen(None);
        }
        if config.minimized {
            window.set_minimized();
        }
        self.state
            .floating_sizes
            .insert(id, (config.width.max(1), config.height.max(1)));
        let qh = self.queue.handle();
        self.state
            .track_aux_scale(SurfaceRole::Floating(id), window.wl_surface(), &qh);
        window.wl_surface().commit();
        self.state.floatings.insert(id, window);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Destroys the current floating window when present.
    pub fn close_floating(&mut self, id: u64) {
        self.state.floatings.remove(&id);
        self.state.aux_scales.remove(&SurfaceRole::Floating(id));
        self.state.floating_sizes.remove(&id);
        self.forget_surface(SurfaceRole::Floating(id));
    }

    pub fn start_floating_move(&self, id: u64) -> bool {
        let (Some(window), Some(seat), Some(serial)) = (
            self.state.floatings.get(&id),
            self.state.pointer_seat.as_ref(),
            self.state.latest_input_serial,
        ) else {
            return false;
        };
        window.move_(seat, serial);
        self.connection.flush().is_ok()
    }

    pub fn start_floating_resize(&self, id: u64, edge: FloatingResizeEdge) -> bool {
        let (Some(window), Some(seat), Some(serial)) = (
            self.state.floatings.get(&id),
            self.state.pointer_seat.as_ref(),
            self.state.latest_input_serial,
        ) else {
            return false;
        };
        window.resize(seat, serial, edge.protocol());
        self.connection.flush().is_ok()
    }

    /// Returns the floating-window surface used to attach buffers.
    pub fn floating_surface(&self, id: u64) -> Option<&wl_surface::WlSurface> {
        self.state.floatings.get(&id).map(Window::wl_surface)
    }

    /// Requests a compositor callback for the next floating-window frame.
    pub fn request_floating_frame(&self, id: u64) {
        let Some(surface) = self.floating_surface(id) else {
            return;
        };
        let qh = self.queue.handle();
        surface.frame(&qh, FrameCallbackData(surface.clone()));
    }

    /// Returns an owned raw-window target for the current floating window.
    pub fn floating_window_target(&self, id: u64) -> Option<WaylandWindowTarget> {
        self.state
            .floatings
            .get(&id)
            .map(|window| WaylandWindowTarget {
                backend: self.connection.backend(),
                surface: window.wl_surface().clone(),
            })
    }
}
