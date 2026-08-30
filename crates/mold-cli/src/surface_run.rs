fn run_surface(
    path: &Path,
    source: &[u8],
    screen: ScreenInfo,
    policy: LoadPolicy,
    tx: &mpsc::Sender<SupervisorMessage>,
    stop: &AtomicBool,
    commands: &mpsc::Receiver<WorkerCommand>,
) -> Result<(), String> {
    let name = screen
        .name
        .clone()
        .ok_or_else(|| format!("output {} has no compositor name", screen.id))?;
    let runtime_screen = Screen {
        id: screen.id,
        name: name.clone(),
        make: screen.make.clone(),
        model: screen.model.clone(),
        description: screen.description.clone(),
        position: screen.position,
        width: screen.size.map(|size| size.0),
        height: screen.size.map(|size| size.1),
        physical_size: screen.physical_size,
        scale: screen.scale,
        transform: screen.transform.to_owned(),
    };
    let mut runtime = Runtime::for_screen(Limits::default(), runtime_screen.clone());
    execute_config(&mut runtime, path, source, policy)?;
    primary_surface_root(&runtime)?;

    let layer_config = runtime.layer_surface_config();
    let mut client = LayerClient::connect(runtime_bar_config(&layer_config, &name)?)
        .map_err(|error| error.to_string())?;
    open_reserve_layers(&mut client, &layer_config, &name)?;

    client.set_idle_timeouts(&runtime.idle_timeouts());
    tx.send(SupervisorMessage::Worker(WorkerMessage::Screens {
        output: name.clone(),
        screens: client.screens().to_vec(),
    }))
    .map_err(|_| "output supervisor stopped".to_owned())?;
    'configured: loop {
        client.dispatch().map_err(|error| error.to_string())?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::Configure { id, .. } if id == PRIMARY_LAYER => break 'configured,
                LayerEvent::Closed { id } if id == PRIMARY_LAYER => {
                    return Err("layer surface was closed".to_owned());
                }
                LayerEvent::Screencopy { request_id, result } => {
                    dispatch_screencopy(&mut runtime, request_id, result);
                }
                LayerEvent::Configure { .. }
                | LayerEvent::Closed { .. }
                | LayerEvent::Scale { .. }
                | LayerEvent::Idle { .. }
                | LayerEvent::OutputPower { .. }
                | LayerEvent::Clipboard { .. }
                | LayerEvent::InputMethod(_)
                | LayerEvent::TextInput(_)
                | LayerEvent::Frame { .. }
                | LayerEvent::PointerMotion { .. }
                | LayerEvent::PointerLeave { .. }
                | LayerEvent::PointerButton { .. }
                | LayerEvent::PointerAxis { .. }
                | LayerEvent::TouchDown { .. }
                | LayerEvent::TouchMotion { .. }
                | LayerEvent::TouchUp { .. }
                | LayerEvent::TouchCancel
                | LayerEvent::Key { .. }
                | LayerEvent::Modifiers { .. }
                | LayerEvent::Screens(_)
                | LayerEvent::PopupConfigure { .. }
                | LayerEvent::PopupFrame { .. }
                | LayerEvent::PopupDone { .. }
                | LayerEvent::FloatingConfigure { .. }
                | LayerEvent::FloatingFrame { .. }
                | LayerEvent::FloatingClose { .. }
                | LayerEvent::SessionLocked
                | LayerEvent::SessionLockFinished
                | LayerEvent::SessionLockConfigure { .. }
                | LayerEvent::SessionLockSurfaceRemoved { .. }
                | LayerEvent::SessionLockFrame { .. } => {}
            }
        }
    }
    let (width, height) = client.physical_size();
    let backend = pollster::block_on(WgpuBackend::new_surface(
        client.window_target(),
        width,
        height,
    ))
    .map_err(|error| error.to_string())?;
    let mut renderer = RenderEngine::new(backend);
    let mut clock = clock_text();
    runtime
        .update_clock(&clock)
        .map_err(|error| error.to_string())?;
    apply_parent_transitions(&mut runtime, &mut renderer, &client)?;
    let layout = paint(&runtime, &mut renderer, &client)?;
    let mut popup_surfaces = HashMap::new();
    let mut floating_surfaces = HashMap::new();
    let mut layer_surfaces = HashMap::new();
    runtime.take_window_surface_change();
    let _ = sync_window_surfaces(
        &runtime,
        &mut client,
        &mut popup_surfaces,
        &mut floating_surfaces,
        &mut layer_surfaces,
        &name,
    )?;
    apply_output_power_requests(&mut runtime, &mut client);
    apply_screencopy_requests(&mut runtime, &mut client);
    apply_virtual_keyboard_requests(&mut runtime, &mut client);
    apply_input_method_requests(&mut runtime, &mut client);
    apply_text_input_requests(&mut runtime, &mut client);

    let mut state = SurfaceEventState {
        layout,
        popup_surfaces,
        floating_surfaces,
        layer_surfaces,
        last_frame: None,
        hovered: None,
        pressed: None,
        focused: HashMap::new(),
        touches: HashMap::new(),
    };
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        client
            .dispatch_timeout(until_next_second().min(Duration::from_millis(100)))
            .map_err(|error| error.to_string())?;
        let next_clock = clock_text();
        let mut repaint = runtime.poll_services();
        let mut recreate_surface = false;
        while let Ok(command) = commands.try_recv() {
            let update = handle_worker_command(&mut runtime, &runtime_screen, policy, command);
            repaint |= update.repaint;
            recreate_surface |= update.recreate_surface;
            if update.reset_input {
                state.hovered = None;
                state.pressed = None;
                state.focused.clear();
                state.touches.clear();
            }
            if update.refresh_idle {
                client.set_idle_timeouts(&runtime.idle_timeouts());
            }
        }
        if recreate_surface {
            let mut replacement = connect_runtime_surface(&runtime, &name)?;
            replacement.set_idle_timeouts(&runtime.idle_timeouts());
            let (width, height) = replacement.physical_size();
            let backend = pollster::block_on(WgpuBackend::new_surface(
                replacement.window_target(),
                width,
                height,
            ))
            .map_err(|error| error.to_string())?;
            renderer = RenderEngine::new(backend);
            state.popup_surfaces.clear();
            state.floating_surfaces.clear();
            state.layer_surfaces.clear();
            client = replacement;
            tx.send(SupervisorMessage::Worker(WorkerMessage::Screens {
                output: name.clone(),
                screens: client.screens().to_vec(),
            }))
            .map_err(|_| "output supervisor stopped".to_owned())?;
        }
        if let Some(hard) = runtime.take_reload_request() {
            tx.send(SupervisorMessage::Reload { hard })
                .map_err(|_| "output supervisor stopped".to_owned())?;
        }
        if let Some(enabled) = runtime.take_watch_files_change() {
            tx.send(SupervisorMessage::WatchFiles(enabled))
                .map_err(|_| "output supervisor stopped".to_owned())?;
        }
        if runtime.take_window_surface_change() {
            repaint |= sync_window_surfaces(
                &runtime,
                &mut client,
                &mut state.popup_surfaces,
                &mut state.floating_surfaces,
                &mut state.layer_surfaces,
                &name,
            )?;
        }
        apply_output_power_requests(&mut runtime, &mut client);
        apply_clipboard_requests(&mut runtime, &mut client);
        apply_screencopy_requests(&mut runtime, &mut client);
        apply_virtual_keyboard_requests(&mut runtime, &mut client);
        apply_input_method_requests(&mut runtime, &mut client);
        apply_text_input_requests(&mut runtime, &mut client);
        if next_clock != clock {
            clock = next_clock;
            runtime
                .update_clock(&clock)
                .map_err(|error| error.to_string())?;
            repaint = true;
        }
        while let Some(event) = client.next_event() {
            repaint |= handle_surface_event(
                &mut runtime,
                &mut renderer,
                &mut client,
                &mut state,
                event,
                tx,
                &name,
            )?;
        }
        apply_clipboard_requests(&mut runtime, &mut client);
        apply_screencopy_requests(&mut runtime, &mut client);
        apply_virtual_keyboard_requests(&mut runtime, &mut client);
        apply_input_method_requests(&mut runtime, &mut client);
        apply_text_input_requests(&mut runtime, &mut client);
        apply_window_surface_actions(&mut runtime, &client, &state.floating_surfaces);
        if repaint {
            apply_parent_transitions(&mut runtime, &mut renderer, &client)?;
            state.layout = paint(&runtime, &mut renderer, &client)?;
            for surface in state
                .popup_surfaces
                .values_mut()
                .filter(|surface| surface.updates_enabled)
            {
                paint_popup_surface(&runtime, &client, surface)?;
            }
            for surface in state
                .floating_surfaces
                .values_mut()
                .filter(|surface| surface.updates_enabled)
            {
                paint_floating_surface(&runtime, &client, surface)?;
            }
            for surface in state
                .layer_surfaces
                .values_mut()
                .filter(|surface| surface.updates_enabled)
            {
                paint_layer_surface(&runtime, &client, surface)?;
            }
        }
    }
}

