/// Identifier of the layer surface every client creates first.
///
/// The role is plural, but one surface is still the shell's own: it is the one
/// `connect` opens, the one whose size and scale the bare accessors report, and
/// the parent an unqualified popup attaches to.
pub const PRIMARY_LAYER: u64 = 0;

impl LayerClient {
    /// Resolves a configured output name against the compositor's current set.
    fn layer_output(
        &self,
        name: Option<&str>,
    ) -> Result<Option<wl_output::WlOutput>, WaylandError> {
        let Some(name) = name else {
            return Ok(None);
        };
        self.state
            .outputs
            .outputs()
            .find(|output| {
                self.state
                    .outputs
                    .info(output)
                    .and_then(|info| info.name)
                    .as_deref()
                    == Some(name)
            })
            .map(Some)
            .ok_or_else(|| WaylandError(format!("Wayland output `{name}` is unavailable")))
    }

    /// Creates or replaces one wlr-layer-shell surface under a client-local id.
    pub fn open_layer(&mut self, id: u64, config: BarConfig) -> Result<(), WaylandError> {
        self.close_layer(id);
        let qh = self.queue.handle();
        let output = self.layer_output(config.output.as_deref())?;
        let surface = self.state.compositor.create_surface(&qh);
        surface.set_buffer_scale(1);
        let layer = self.state.layer_shell.create_layer_surface(
            &qh,
            surface,
            match config.layer {
                ShellLayer::Background => Layer::Background,
                ShellLayer::Bottom => Layer::Bottom,
                ShellLayer::Top => Layer::Top,
                ShellLayer::Overlay => Layer::Overlay,
            },
            Some(config.namespace),
            output.as_ref(),
        );
        let mut anchors = Anchor::empty();
        if config.anchors.top {
            anchors |= Anchor::TOP;
        }
        if config.anchors.right {
            anchors |= Anchor::RIGHT;
        }
        if config.anchors.bottom {
            anchors |= Anchor::BOTTOM;
        }
        if config.anchors.left {
            anchors |= Anchor::LEFT;
        }
        layer.set_anchor(anchors);
        layer.set_keyboard_interactivity(match config.keyboard_focus {
            KeyboardFocus::None => WlrKeyboardInteractivity::None,
            KeyboardFocus::Exclusive => WlrKeyboardInteractivity::Exclusive,
            KeyboardFocus::OnDemand => WlrKeyboardInteractivity::OnDemand,
        });
        layer.set_size(config.width, config.height);
        layer.set_margin(
            config.margin_top,
            config.margin_right,
            config.margin_bottom,
            config.margin_left,
        );
        layer.set_exclusive_zone(config.exclusive_zone);
        let fractional_scale = self
            .state
            .fractional_manager
            .as_ref()
            .map(|manager| manager.get_fractional_scale(layer.wl_surface(), &qh, id));
        let viewport = self
            .state
            .viewporter
            .as_ref()
            .map(|manager| manager.get_viewport(layer.wl_surface(), &qh, ()));
        layer.commit();
        self.state.layers.insert(
            id,
            LayerRecord {
                surface: layer,
                fractional_scale,
                viewport,
                width: 1,
                height: config.height.max(1),
                scale_120: 120,
                wants_blank: false,
                configured: false,
                blank: None,
            },
        );
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Asks a layer surface to map itself with a single transparent pixel.
    ///
    /// A surface that never attaches a buffer stays unmapped, and a compositor
    /// derives an output's usable area only from the layer surfaces it actually
    /// arranges — so an unmapped reserver reserves nothing at all. The protocol
    /// requires the first commit to carry no buffer and the configure that
    /// follows to be acknowledged before one may be attached, so this records
    /// the intent and the configure handler completes it.
    pub fn map_layer_blank(&mut self, id: u64) -> Result<(), WaylandError> {
        let Some(record) = self.state.layers.get_mut(&id) else {
            return Ok(());
        };
        if record.blank.is_some() {
            return Ok(());
        }
        record.wants_blank = true;
        self.state.attach_blank_buffer(id);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Destroys one layer surface when it is open.
    pub fn close_layer(&mut self, id: u64) {
        if self.state.layers.remove(&id).is_none() {
            return;
        }
        if self.state.keyboard_surface == Some(SurfaceRole::Layer(id)) {
            self.state.keyboard_surface = None;
        }
        self.state
            .touch_points
            .retain(|_, (_, role)| *role != SurfaceRole::Layer(id));
    }

    /// Reports whether one layer surface is currently open.
    pub fn layer_is_open(&self, id: u64) -> bool {
        self.state.layers.contains_key(&id)
    }

    /// Returns the wl_surface backing one layer surface.
    pub fn layer_surface(&self, id: u64) -> Option<&wl_surface::WlSurface> {
        self.state
            .layers
            .get(&id)
            .map(|layer| layer.surface.wl_surface())
    }

    /// Returns the configured logical dimensions of one layer surface.
    pub fn layer_logical_size(&self, id: u64) -> Option<(u32, u32)> {
        self.state
            .layers
            .get(&id)
            .map(|layer| (layer.width, layer.height))
    }

    /// Returns the preferred scale of one layer surface in 120ths.
    pub fn layer_scale_120(&self, id: u64) -> Option<u32> {
        self.state.layers.get(&id).map(|layer| layer.scale_120)
    }

    /// Returns the physical buffer dimensions of one layer surface.
    pub fn layer_physical_size(&self, id: u64) -> Option<(u32, u32)> {
        let layer = self.state.layers.get(&id)?;
        Some(physical_size((layer.width, layer.height), layer.scale_120))
    }

    /// Requests a compositor callback for one layer surface's next frame.
    pub fn request_layer_frame(&self, id: u64) {
        let Some(surface) = self.layer_surface(id) else {
            return;
        };
        let qh = self.queue.handle();
        surface.frame(&qh, FrameCallbackData(surface.clone()));
    }

    /// Commits pending state on one layer surface without attaching a buffer.
    pub fn commit_layer(&self, id: u64) {
        if let Some(surface) = self.layer_surface(id) {
            surface.commit();
        }
    }

    /// Returns an owned raw-window target for one layer surface.
    pub fn layer_window_target(&self, id: u64) -> Option<WaylandWindowTarget> {
        self.layer_surface(id).map(|surface| WaylandWindowTarget {
            backend: self.connection.backend(),
            surface: surface.clone(),
        })
    }

    /// Applies the default, empty, or rectangular input region to one surface.
    pub fn set_layer_input_region(&self, id: u64, rectangles: Option<&[InputRect]>) {
        let Some(surface) = self.layer_surface(id) else {
            return;
        };
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

    /// Builds and applies a composable logical input region to one surface.
    pub fn set_layer_composed_input_region(
        &self,
        id: u64,
        regions: &[Region],
    ) -> Result<(), WaylandError> {
        let (width, height) = self
            .layer_logical_size(id)
            .ok_or_else(|| WaylandError("layer surface is not open".into()))?;
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
        self.set_layer_input_region(id, Some(&rectangles));
        Ok(())
    }
}
