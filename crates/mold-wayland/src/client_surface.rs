use crate::client_layer::PRIMARY_LAYER;
use smithay_client_toolkit::compositor::FrameCallbackData;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::XdgPositioner;
use smithay_client_toolkit::shell::xdg::XdgSurface;
use smithay_client_toolkit::shell::xdg::popup::Popup;
use std::collections::HashMap;
use wayland_client::Proxy;
use wayland_client::protocol::wl_surface;

use crate::{helpers::*, state_types::*, surface_types::*, types::*};

/// First `xdg_popup` version carrying the `reposition` request.
pub(crate) const XDG_POPUP_REPOSITION_VERSION: u32 = 3;

/// Reposition bookkeeping for one popup.
///
/// Only the counter. The token the compositor echoed back was recorded here too
/// and read by nothing — the configure it arrives with already carries it, so
/// the copy in this map answered a question nobody asked.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PopupReposition {
    /// Last token sent with `xdg_popup.reposition`.
    pub(crate) sent: u32,
}

/// Issues the next reposition token for one popup.
///
/// The token is opaque to the compositor and only has to identify the request
/// it came from, so it is a per-popup counter starting at one; zero is left
/// unused so a fresh counter never looks like an acknowledged request.
pub(crate) fn next_reposition_token(
    repositions: &mut HashMap<u64, PopupReposition>,
    id: u64,
) -> u32 {
    let reposition = repositions.entry(id).or_default();
    reposition.sent = reposition.sent.wrapping_add(1).max(1);
    reposition.sent
}

impl LayerClient {
    /// Removes the next queued surface event.
    pub fn next_event(&mut self) -> Option<LayerEvent> {
        while let Ok(text) = self.state.clipboard_rx.try_recv() {
            self.state.events.push_back(LayerEvent::Clipboard { text });
        }
        self.state.events.pop_front()
    }

    /// Requests a compositor callback for the next frame.
    pub fn request_frame(&self) {
        self.request_layer_frame(PRIMARY_LAYER);
    }

    /// Returns the underlying surface used to construct a GPU presentation target.
    pub fn surface(&self) -> &wl_surface::WlSurface {
        self.state.layer().wl_surface()
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
        self.layer_logical_size(PRIMARY_LAYER).unwrap_or((1, 1))
    }

    /// Returns the preferred scale in 120ths.
    pub fn scale_120(&self) -> u32 {
        self.layer_scale_120(PRIMARY_LAYER).unwrap_or(120)
    }

    /// Returns the physical buffer dimensions rounded upward.
    pub fn physical_size(&self) -> (u32, u32) {
        physical_size(self.logical_size(), self.scale_120())
    }

    /// Returns the latest compositor output snapshot.
    pub fn screens(&self) -> &[ScreenInfo] {
        &self.state.screens
    }

    /// Builds the positioner describing one popup's requested geometry.
    ///
    /// Every field a positioner carries is replaceable on a live popup:
    /// `xdg_popup.reposition` discards the previous positioner wholesale
    /// (`xdg-shell.xml:1368-1374`), so the same builder serves both creating a
    /// popup and moving one.
    pub(crate) fn build_positioner(
        &self,
        config: &PopupConfig,
    ) -> Result<XdgPositioner, WaylandError> {
        let positioner = XdgPositioner::new(&self.state.xdg_shell)
            .map_err(|error| WaylandError(format!("could not create popup positioner: {error}")))?;
        positioner.set_size(config.width.max(1) as i32, config.height.max(1) as i32);
        positioner.set_anchor_rect(
            config.anchor.x,
            config.anchor.y,
            config.anchor.width.max(1),
            config.anchor.height.max(1),
        );
        positioner.set_anchor(popup_anchor(config.anchor_edge));
        positioner.set_gravity(popup_gravity(config.gravity));
        positioner.set_offset(config.offset_x, config.offset_y);
        positioner.set_constraint_adjustment(popup_constraints(config.constraints));
        Ok(positioner)
    }

    /// Creates an xdg popup anchored to a parent-surface rectangle.
    pub fn open_popup(
        &mut self,
        id: u64,
        parent: SurfaceRole,
        config: PopupConfig,
    ) -> Result<(), WaylandError> {
        self.close_popup(id);
        let qh = self.queue.handle();
        let positioner = self.build_positioner(&config)?;
        let surface = self.state.compositor.create_surface(&qh);
        surface.set_buffer_scale(1);
        let parent_surface = match parent {
            SurfaceRole::Layer(_) => None,
            SurfaceRole::Popup(parent) => Some(
                self.state
                    .popups
                    .get(&parent)
                    .ok_or_else(|| WaylandError("popup parent is not open".into()))?
                    .xdg_surface(),
            ),
            SurfaceRole::Floating(parent) => Some(
                self.state
                    .floatings
                    .get(&parent)
                    .ok_or_else(|| WaylandError("popup parent is not open".into()))?
                    .xdg_surface(),
            ),
        };
        let popup = Popup::from_surface(
            parent_surface,
            &positioner,
            &qh,
            surface,
            &self.state.xdg_shell,
        )
        .map_err(|error| WaylandError(format!("could not create popup: {error}")))?;
        if let SurfaceRole::Layer(id) = parent {
            self.state
                .layers
                .get(&id)
                .ok_or_else(|| WaylandError("popup parent layer is not open".into()))?
                .surface
                .get_popup(popup.xdg_popup());
        }
        if config.grab_focus {
            let seat = self
                .state
                .pointer_seat
                .as_ref()
                .ok_or_else(|| WaylandError("popup grab requires a pointer seat".into()))?;
            let serial = self
                .state
                .latest_input_serial
                .ok_or_else(|| WaylandError("popup grab requires an input serial".into()))?;
            popup.xdg_popup().grab(seat, serial);
        }
        popup.wl_surface().commit();
        self.state.popups.insert(id, popup);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Moves or resizes a mapped popup in place.
    ///
    /// `xdg_popup.reposition` replaces the popup's positioner without touching
    /// the surface, so the wl_surface, its GPU surface and its swapchain all
    /// survive a move. The compositor answers with `repositioned` carrying the
    /// token, then a configure; the new geometry takes effect once that
    /// configure is acknowledged (`xdg-shell.xml:1368-1380`).
    ///
    /// Returns `false` — having changed nothing — when the compositor's
    /// `xdg_popup` predates version 3 and has no `reposition` request at all,
    /// so the caller can fall back to closing and reopening the popup.
    pub fn reposition_popup(&mut self, id: u64, config: PopupConfig) -> Result<bool, WaylandError> {
        let version = self
            .state
            .popups
            .get(&id)
            .ok_or_else(|| WaylandError("popup is not open".into()))?
            .xdg_popup()
            .version();
        if version < XDG_POPUP_REPOSITION_VERSION {
            return Ok(false);
        }
        let positioner = self.build_positioner(&config)?;
        let token = next_reposition_token(&mut self.state.popup_repositions, id);
        let Some(popup) = self.state.popups.get(&id) else {
            return Ok(false);
        };
        popup.xdg_popup().reposition(&positioner, token);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))?;
        Ok(true)
    }

    /// Destroys the current popup when present.
    pub fn close_popup(&mut self, id: u64) {
        self.state.popups.remove(&id);
        self.state.popup_repositions.remove(&id);
        self.forget_surface(SurfaceRole::Popup(id));
    }

    /// Returns the popup surface used to attach buffers.
    pub fn popup_surface(&self, id: u64) -> Option<&wl_surface::WlSurface> {
        self.state.popups.get(&id).map(Popup::wl_surface)
    }

    /// Requests a compositor callback for the next popup frame.
    pub fn request_popup_frame(&self, id: u64) {
        let Some(surface) = self.popup_surface(id) else {
            return;
        };
        let qh = self.queue.handle();
        surface.frame(&qh, FrameCallbackData(surface.clone()));
    }

    /// Returns an owned raw-window target for the current popup.
    pub fn popup_window_target(&self, id: u64) -> Option<WaylandWindowTarget> {
        self.state.popups.get(&id).map(|popup| WaylandWindowTarget {
            backend: self.connection.backend(),
            surface: popup.wl_surface().clone(),
        })
    }
}
