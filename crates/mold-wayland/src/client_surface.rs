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

    /// Builds and applies a composable logical input region.
    pub fn set_composed_input_region(&self, regions: &[Region]) -> Result<(), WaylandError> {
        let (width, height) = self.logical_size();
        let rectangles = mold_region::build(width, height, regions)
            .map_err(|error| WaylandError(error.to_string()))?
            .into_iter()
            .map(|rect| InputRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            })
            .collect::<Vec<_>>();
        self.set_input_region(Some(&rectangles));
        Ok(())
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
        let surface = self.state.compositor.create_surface(&qh);
        surface.set_buffer_scale(1);
        let parent_surface = match parent {
            SurfaceRole::Layer => None,
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
        if parent == SurfaceRole::Layer {
            self.state.layer().get_popup(popup.xdg_popup());
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

    /// Destroys the current popup when present.
    pub fn close_popup(&mut self, id: u64) {
        self.state.popups.remove(&id);
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

