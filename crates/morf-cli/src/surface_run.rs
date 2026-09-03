use morf_lua::{Limits, Runtime, Screen};
use morf_render::{RenderEngine, ShaderRegistration, WgpuBackend};
use morf_wayland::{LayerClient, LayerEvent, PRIMARY_LAYER, ScreenInfo};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::{
    capture::*, config::*, lock::*, pacing::*, paint::*, services::*, supervisor::*,
    surface_actions::*, surface_events::*, surface_layers::*, surfaces::*, workers::*,
};

/// What this output can do, as name = value pairs.
///
/// Booleans for the protocols, because "is there screencopy here" is the
/// question; strings for the GPU, because "which one" is.
fn capabilities_of(
    client: &LayerClient,
    renderer: &mut RenderEngine<WgpuBackend>,
) -> Vec<(String, String)> {
    let info = renderer.backend_mut().info();
    let mut list = vec![
        ("gpu".to_owned(), info.name.clone()),
        ("gpu_backend".to_owned(), format!("{:?}", info.backend)),
        ("scale_120".to_owned(), client.scale_120().to_string()),
    ];
    for (name, supported) in [
        ("clipboard", client.supports_clipboard()),
        ("virtual_keyboard", client.supports_virtual_keyboard()),
        ("input_method", client.supports_input_method()),
        ("text_input", client.supports_text_input()),
        ("screencopy", client.supports_screencopy()),
        ("image_capture", client.supports_image_capture()),
        ("window_capture", client.supports_window_capture()),
        (
            "dmabuf_capture",
            client.supports_dmabuf_capture() && info.dmabuf,
        ),
        ("backdrop_blur", client.supports_backdrop_blur()),
        ("toplevels", client.supports_toplevels()),
        ("toplevel_control", client.supports_toplevel_control()),
    ] {
        list.push((name.to_owned(), supported.to_string()));
    }
    list
}

pub(crate) fn run_surface(
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
    // What the reservers were last built from. A reserver is a separate surface
    // per edge, so a thickness change is the one part of `morf.surface` that
    // still has to rebuild something, and it must not rebuild on every
    // unrelated margin the configuration animates.
    let mut reserve = layer_config.reserve;

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
                    dispatch_screencopy(&mut runtime, None, request_id, result);
                }
                LayerEvent::CaptureOffer {
                    request_id,
                    width,
                    height,
                    device,
                    formats,
                } => {
                    // No renderer yet to export against: shared memory.
                    answer_capture_offer(
                        &mut runtime,
                        None,
                        &mut client,
                        OfferedCapture {
                            request_id,
                            width,
                            height,
                            device,
                            formats,
                        },
                    );
                }
                LayerEvent::Configure { .. }
                | LayerEvent::Closed { .. }
                | LayerEvent::Scale { .. }
                | LayerEvent::AuxScale { .. }
                | LayerEvent::ShortcutsInhibited { .. }
                | LayerEvent::Idle { .. }
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
    // Known only now: the protocols came with the connection, the GPU with
    // the renderer. Everything a configuration or `morf info` might ask.
    runtime.set_capabilities(&capabilities_of(&client, &mut renderer));
    register_shaders(&runtime, &mut renderer)?;
    let animating_shaders = runtime.shaders_animate();
    let started = Instant::now();
    let mut clock = clock_text();
    runtime
        .update_clock(&clock)
        .map_err(|error| error.to_string())?;
    apply_parent_transitions(&mut runtime, &mut renderer, &client)?;
    let primary_root = primary_surface_root(&runtime)?;
    let layout = paint(&runtime, &mut renderer, &client, primary_root, None)?;
    let mut popup_surfaces = HashMap::new();
    let mut floating_surfaces = HashMap::new();
    let mut layer_surfaces = HashMap::new();
    runtime.take_window_surface_change();
    runtime.take_layer_surface_change();
    let _ = sync_window_surfaces(
        &runtime,
        &mut client,
        &mut popup_surfaces,
        &mut floating_surfaces,
        &mut layer_surfaces,
        &name,
    )?;
    apply_service_requests(&mut runtime, &mut client);

    let mut state = SurfaceEventState {
        layout,
        primary_root,
        popup_surfaces,
        floating_surfaces,
        layer_surfaces,
        animating_shaders,
        last_frame: None,
        pacer: FramePacer::new(),
        // Until a callback says otherwise, assume the commonest refresh.
        refresh: Duration::from_micros(16_667),
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
            // The adapter is new, so every pipeline it held is gone with it.
            register_shaders(&runtime, &mut renderer)?;
            state.popup_surfaces.clear();
            state.floating_surfaces.clear();
            state.layer_surfaces.clear();
            client = replacement;
            reserve = runtime.layer_surface_config().reserve;
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
        if runtime.quit_requested() {
            // Told once, and then this output stops driving frames. The
            // supervisor takes the others down; returning here rather than
            // waiting for it keeps this thread from painting a shell that is
            // already leaving.
            tx.send(SupervisorMessage::Quit)
                .map_err(|_| "output supervisor stopped".to_owned())?;
            return Ok(());
        }
        apply_idle_inhibit(&mut runtime, &mut client);
        apply_shortcuts_inhibit(&mut runtime, &mut client);
        if let Some(enabled) = runtime.take_watch_files_change() {
            tx.send(SupervisorMessage::WatchFiles(enabled))
                .map_err(|_| "output supervisor stopped".to_owned())?;
        }
        if runtime.take_layer_surface_change() {
            // Layer shell allows all of this on a mapped surface, so the shell's
            // own geometry follows an assignment to `morf.surface` without a
            // reconnect. The configure this provokes resizes the backend in
            // place; nothing here tears the renderer down.
            let config = runtime.layer_surface_config();
            client
                .set_layer_geometry(PRIMARY_LAYER, &runtime_bar_config(&config, &name)?)
                .map_err(|error| error.to_string())?;
            if config.reserve != reserve {
                reserve = config.reserve;
                open_reserve_layers(&mut client, &config, &name)?;
            }
            apply_primary_opaque(&runtime, &client);
            // The mask lives in the same configuration and is re-derived when
            // the surface paints, so the new geometry owes one frame even when
            // the compositor has no configure to send back.
            repaint = true;
        }
        if runtime.take_window_surface_change() {
            // The only thing that can move the primary root.
            state.primary_root = primary_surface_root(&runtime)?;
            repaint |= sync_window_surfaces(
                &runtime,
                &mut client,
                &mut state.popup_surfaces,
                &mut state.floating_surfaces,
                &mut state.layer_surfaces,
                &name,
            )?;
        }
        apply_service_requests(&mut runtime, &mut client);
        if next_clock != clock {
            clock = next_clock;
            repaint |= runtime
                .update_clock(&clock)
                .map_err(|error| error.to_string())?;
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
        apply_service_requests(&mut runtime, &mut client);
        apply_capture_releases(&mut runtime, &mut renderer);
        apply_window_surface_actions(&mut runtime, &client, &state.floating_surfaces);
        if repaint {
            let painted = Instant::now();
            renderer
                .backend_mut()
                .set_elapsed(started.elapsed().as_secs_f32());
            // Before anything is drawn, tell the renderer what died. Its caches
            // are keyed on nodes and it has no other way to find out; without
            // this a shaped text buffer survives every view switch for the life
            // of the process.
            let removed = runtime.take_removed_nodes();
            if !removed.is_empty() {
                renderer.backend_mut().forget_nodes(&removed);
            }
            apply_parent_transitions(&mut runtime, &mut renderer, &client)?;
            state.layout = paint(
                &runtime,
                &mut renderer,
                &client,
                state.primary_root,
                Some(&state.layout),
            )?;
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
            // What this frame actually cost, which is what the next one is
            // paced against.
            state.pacer.observed(painted.elapsed());
        }
    }
}

/// Builds a pipeline for every shader the configuration registered.
///
/// Once, at startup and after a device loss — never during a frame. Compiling a
/// pipeline costs tens of milliseconds, which is several frames' worth of
/// budget, and a shader is known the moment the configuration finishes loading.
fn register_shaders(
    runtime: &Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
) -> Result<(), String> {
    for shader in runtime.shaders() {
        renderer
            .backend_mut()
            .register_shader(ShaderRegistration {
                program: shader.program,
                wgsl: Some(&shader.wgsl),
                vertex: shader.vertex.as_deref(),
                offsets: &shader.offsets,
                uniform_size: shader.uniform_size,
                owns_coverage: shader.owns_coverage,
                effect: shader.samples_behind,
                textures: &shader.textures,
                data: &shader.data,
            })
            .map_err(|error| format!("shader pipeline: {error}"))?;
    }
    Ok(())
}
