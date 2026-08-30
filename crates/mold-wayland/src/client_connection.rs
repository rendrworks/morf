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
        let text_input_manager = globals
            .bind::<ZwpTextInputManagerV3, _, _>(&qh, 1..=2, ())
            .ok();
        let output_power_manager = globals
            .bind::<ZwlrOutputPowerManagerV1, _, _>(&qh, 1..=1, ())
            .ok();
        let shm = Shm::bind(&globals, &qh).ok();
        let screencopy_manager = globals
            .bind::<ZwlrScreencopyManagerV1, _, _>(&qh, 1..=3, ())
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
            popups: HashMap::new(),
            floatings: HashMap::new(),
            floating_sizes: HashMap::new(),
            _fractional_manager: fractional_manager,
            fractional_scale: None,
            _viewporter: viewporter,
            viewport: None,
            width: 1,
            height: config.as_ref().map_or(1, |config| config.height.max(1)),
            scale_120: 120,
            events: VecDeque::new(),
            pointer: None,
            pointer_seat: None,
            keyboard: None,
            touch: None,
            touch_points: HashMap::new(),
            keyboard_surface: None,
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
}

