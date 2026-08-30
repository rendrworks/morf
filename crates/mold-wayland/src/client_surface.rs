/// First `xdg_popup` version carrying the `reposition` request.
const XDG_POPUP_REPOSITION_VERSION: u32 = 3;

/// Reposition bookkeeping for one popup.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PopupReposition {
    /// Last token sent with `xdg_popup.reposition`.
    sent: u32,
    /// Last token the compositor echoed back with `xdg_popup.repositioned`.
    acknowledged: Option<u32>,
}

/// Issues the next reposition token for one popup.
///
/// The token is opaque to the compositor and only has to identify the request
/// it came from, so it is a per-popup counter starting at one; zero is left
/// unused so a fresh counter never looks like an acknowledged request.
fn next_reposition_token(repositions: &mut HashMap<u64, PopupReposition>, id: u64) -> u32 {
    let reposition = repositions.entry(id).or_default();
    reposition.sent = reposition.sent.wrapping_add(1).max(1);
    reposition.sent
}

/// Records the token the compositor echoed for a popup it has repositioned.
fn record_reposition_ack(repositions: &mut HashMap<u64, PopupReposition>, id: u64, token: u32) {
    repositions.entry(id).or_default().acknowledged = Some(token);
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

    /// Commits pending surface state without attaching a buffer.
    pub fn commit(&self) {
        self.commit_layer(PRIMARY_LAYER);
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

    /// Applies the default, empty, or explicitly rectangular input region.
    pub fn set_input_region(&self, rectangles: Option<&[InputRect]>) {
        self.set_layer_input_region(PRIMARY_LAYER, rectangles);
    }

    /// Builds and applies a composable logical input region.
    pub fn set_composed_input_region(&self, regions: &[Region]) -> Result<(), WaylandError> {
        self.set_layer_composed_input_region(PRIMARY_LAYER, regions)
    }

    /// Builds the positioner describing one popup's requested geometry.
    ///
    /// Every field a positioner carries is replaceable on a live popup:
    /// `xdg_popup.reposition` discards the previous positioner wholesale
    /// (`xdg-shell.xml:1368-1374`), so the same builder serves both creating a
    /// popup and moving one.
    fn build_positioner(&self, config: &PopupConfig) -> Result<XdgPositioner, WaylandError> {
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

    /// Returns the reposition token the compositor last echoed for this popup.
    ///
    /// A caller that issued several repositions correlates an arriving
    /// [`LayerEvent::PopupConfigure`] with the request it answers by comparing
    /// this against the request order; it is `None` until the first
    /// `xdg_popup.repositioned` for the popup arrives.
    pub fn popup_reposition_token(&self, id: u64) -> Option<u32> {
        self.state
            .popup_repositions
            .get(&id)
            .and_then(|reposition| reposition.acknowledged)
    }

    /// Destroys the current popup when present.
    pub fn close_popup(&mut self, id: u64) {
        self.state.popups.remove(&id);
        self.state.popup_repositions.remove(&id);
        if self.state.keyboard_surface == Some(SurfaceRole::Popup(id)) {
            self.state.keyboard_surface = None;
        }
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
