use smithay_client_toolkit::compositor::CompositorHandler;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::session_lock::{
    SessionLock, SessionLockHandler, SessionLockSurface, SessionLockSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::xdg::popup::{Popup, PopupConfigure, PopupHandler};
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use std::num::NonZeroU32;
use wayland_client::protocol::{wl_output, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::{state_types::*, surface_types::*};

impl CompositorHandler for LayerState {
    fn scale_factor_changed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        let Some(id) = self.layer_id(surface) else {
            return;
        };
        let Some(layer) = self
            .layers
            .get_mut(&id)
            .filter(|layer| layer.fractional_scale.is_none())
        else {
            return;
        };
        layer.scale_120 = factor.max(1) as u32 * 120;
        let scale_120 = layer.scale_120;
        self.events.push_back(LayerEvent::Scale { id, scale_120 });
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
        if let Some(id) = self.layer_id(surface) {
            self.events
                .push_back(LayerEvent::Frame { id, time_ms: time });
        } else if let Some(id) = self
            .popups
            .iter()
            .find_map(|(id, popup)| (surface == popup.wl_surface()).then_some(*id))
        {
            self.events
                .push_back(LayerEvent::PopupFrame { id, time_ms: time });
        } else if let Some(id) = self
            .floatings
            .iter()
            .find_map(|(id, window)| (surface == window.wl_surface()).then_some(*id))
        {
            self.events
                .push_back(LayerEvent::FloatingFrame { id, time_ms: time });
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
    fn closed(&mut self, _connection: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        let Some(id) = self.layer_id(layer.wl_surface()) else {
            return;
        };
        self.events.push_back(LayerEvent::Closed { id });
    }

    fn configure(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(id) = self.layer_id(layer.wl_surface()) else {
            return;
        };
        let Some(record) = self.layers.get_mut(&id) else {
            return;
        };
        record.width = NonZeroU32::new(configure.new_size.0).map_or(record.width, NonZeroU32::get);
        record.height =
            NonZeroU32::new(configure.new_size.1).map_or(record.height, NonZeroU32::get);
        if let Some(viewport) = &record.viewport {
            viewport.set_destination(record.width as i32, record.height as i32);
        }
        record.configured = true;
        let (width, height) = (record.width, record.height);
        // sctk acknowledges the configure for us, so a surface waiting to map
        // itself may attach its buffer now.
        self.attach_blank_buffer(id);
        self.events
            .push_back(LayerEvent::Configure { id, width, height });
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
        popup: &Popup,
        config: PopupConfigure,
    ) {
        let Some(id) = self.popups.iter().find_map(|(id, candidate)| {
            (candidate.wl_surface() == popup.wl_surface()).then_some(*id)
        }) else {
            return;
        };
        // A configure that answers a reposition carries the token of the
        // request it answers. Recording it here is what lets a caller tell the
        // configure for the move it just asked for apart from a reactive one
        // the compositor sent on its own.
        self.events.push_back(LayerEvent::PopupConfigure {
            id,
            width: config.width.max(1) as u32,
            height: config.height.max(1) as u32,
        });
    }

    fn done(&mut self, _connection: &Connection, _qh: &QueueHandle<Self>, popup: &Popup) {
        let Some(id) = self.popups.iter().find_map(|(id, candidate)| {
            (candidate.wl_surface() == popup.wl_surface()).then_some(*id)
        }) else {
            return;
        };
        self.popups.remove(&id);
        self.popup_repositions.remove(&id);
        self.events.push_back(LayerEvent::PopupDone { id });
    }
}

impl WindowHandler for LayerState {
    fn request_close(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        window: &Window,
    ) {
        let Some(id) = self.floatings.iter().find_map(|(id, candidate)| {
            (candidate.wl_surface() == window.wl_surface()).then_some(*id)
        }) else {
            return;
        };
        self.floatings.remove(&id);
        self.floating_sizes.remove(&id);
        self.events.push_back(LayerEvent::FloatingClose { id });
    }

    fn configure(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let Some(id) = self.floatings.iter().find_map(|(id, candidate)| {
            (candidate.wl_surface() == window.wl_surface()).then_some(*id)
        }) else {
            return;
        };
        let previous = self.floating_sizes.get(&id).copied().unwrap_or((1, 1));
        let width = configure.new_size.0.map_or(previous.0, NonZeroU32::get);
        let height = configure.new_size.1.map_or(previous.1, NonZeroU32::get);
        self.floating_sizes.insert(id, (width, height));
        self.events
            .push_back(LayerEvent::FloatingConfigure { id, width, height });
    }
}
