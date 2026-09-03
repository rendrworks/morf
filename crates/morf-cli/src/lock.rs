use morf_io::IpcIncoming;
use morf_layout::{Layout, Size};
use morf_lua::{IpcValue, Runtime};
use morf_render::{RenderEngine, WgpuBackend};
use morf_scene::{Element, NodeHandle};
use morf_wayland::{LayerClient, LayerEvent, ScreenInfo};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::{capture::*, config::*, paint::*, supervisor::*, surface_layers::*, surfaces::*};

pub(crate) struct Worker {
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) commands: mpsc::Sender<WorkerCommand>,
    pub(crate) join: JoinHandle<()>,
    pub(crate) screen: ScreenInfo,
}

pub(crate) enum WorkerCommand {
    Call {
        target: String,
        args: Vec<IpcValue>,
        reply: mpsc::SyncSender<Result<Vec<IpcValue>, String>>,
    },
    /// The compositor's output list, as the supervisor last recorded it.
    Screens(Vec<ScreenInfo>),
    Verbs(mpsc::SyncSender<Vec<String>>),
    Logs(mpsc::SyncSender<Vec<String>>),
    Capabilities(mpsc::SyncSender<Vec<String>>),
    Bindings(mpsc::SyncSender<Vec<String>>),
    Reload {
        path: Arc<PathBuf>,
        source: Arc<[u8]>,
        hard: bool,
        reply: mpsc::SyncSender<Result<(), String>>,
    },
}

pub(crate) enum SupervisorMessage {
    Worker(WorkerMessage),
    Ipc(IpcIncoming),
    Reload {
        hard: bool,
    },
    WatchFiles(bool),
    /// The configuration asked the shell to stop.
    Quit,
}

pub(crate) enum WorkerMessage {
    Screens {
        output: String,
        screens: Vec<ScreenInfo>,
    },
    Failed {
        output: String,
        error: String,
    },
}

pub(crate) fn run_lock(path: &Path, source: &[u8]) -> Result<(), String> {
    let mut runtime = Runtime::default();
    execute_config(&mut runtime, path, source, LoadPolicy::default())?;
    if runtime.scene().roots().len() != 1 {
        return Err("lock configuration must create exactly one root item".to_owned());
    }
    let root = runtime.scene().roots()[0];
    if runtime
        .scene()
        .element(root)
        .map_err(|error| error.to_string())?
        != Element::Rect
    {
        return Err("lock configuration root must be an opaque Rect".to_owned());
    }
    let mut client = LayerClient::connect_lock().map_err(|error| error.to_string())?;
    client.set_idle_timeouts(&runtime.idle_timeouts());
    client
        .begin_session_lock()
        .map_err(|error| error.to_string())?;
    apply_service_requests(&mut runtime, &mut client);
    let mut renderers: Vec<Option<RenderEngine<WgpuBackend>>> = Vec::new();
    let mut last_frame = None;
    // Which node holds keyboard focus. One surface, so one slot rather than the
    // per-surface map the general path keeps.
    let mut focused: Option<NodeHandle> = None;
    let mut locked = false;
    let mut unlock_pending = false;
    let mut clock = clock_text();
    runtime
        .update_clock(&clock)
        .map_err(|error| error.to_string())?;
    loop {
        client
            .dispatch_timeout(until_next_second().min(Duration::from_millis(100)))
            .map_err(|error| error.to_string())?;
        let mut repaint = runtime.poll_services();
        apply_service_requests(&mut runtime, &mut client);
        unlock_pending |= runtime.take_session_unlock_request();
        if locked && unlock_pending {
            client.unlock_session().map_err(|error| error.to_string())?;
            return Ok(());
        }
        let next_clock = clock_text();
        if next_clock != clock {
            clock = next_clock;
            repaint |= runtime
                .update_clock(&clock)
                .map_err(|error| error.to_string())?;
        }
        while let Some(event) = client.next_event() {
            match event {
                // A lock client has only lock surfaces, and those are not
                // popups or floating windows.
                LayerEvent::AuxScale { .. } | LayerEvent::ShortcutsInhibited { .. } => {}
                LayerEvent::SessionLocked => locked = true,
                LayerEvent::Screens(_) => {}
                LayerEvent::SessionLockConfigure { index, .. } => {
                    renderers.resize_with(index + 1, || None);
                    let (width, height) = client
                        .lock_physical_size(index)
                        .ok_or_else(|| "configured lock surface disappeared".to_owned())?;
                    if let Some(renderer) = &mut renderers[index] {
                        renderer.resize(width, height);
                    } else {
                        let target = client
                            .lock_window_target(index)
                            .ok_or_else(|| "configured lock surface disappeared".to_owned())?;
                        let backend =
                            pollster::block_on(WgpuBackend::new_surface(target, width, height))
                                .map_err(|error| error.to_string())?;
                        renderers[index] = Some(RenderEngine::new(backend));
                    }
                    repaint = true;
                }
                LayerEvent::SessionLockSurfaceRemoved { index } => {
                    if index < renderers.len() {
                        renderers.remove(index);
                    }
                }
                LayerEvent::SessionLockFrame { time_ms, .. } => {
                    let frame = runtime
                        .tick_animations(animation_delta(last_frame, time_ms))
                        .map_err(|error| error.to_string())?;
                    last_frame = frame.active.then_some(time_ms);
                    repaint |= frame.active || frame.changed > 0;
                }
                LayerEvent::Key {
                    pressed: true,
                    keysym,
                    text,
                    ..
                } => {
                    // The same routing every other surface gets. This used to
                    // send every key to the first focusable node in the tree,
                    // with no Tab traversal and no memory of what had focus —
                    // so a lock screen with more than one field had one that
                    // could not be reached.
                    let Some(root) = runtime.scene().roots().first().copied() else {
                        continue;
                    };
                    repaint |= dispatch_key_in_subtree(
                        &mut runtime,
                        root,
                        &mut focused,
                        keysym,
                        text.as_deref(),
                    );
                }
                LayerEvent::SessionLockFinished => {
                    return Err("compositor ended the session lock".to_owned());
                }
                LayerEvent::Idle {
                    timeout_ms,
                    input_only,
                    idle,
                } => {
                    repaint |= runtime.dispatch_idle(timeout_ms, input_only, idle);
                }
                LayerEvent::Clipboard { text } => {
                    repaint |= runtime.dispatch_clipboard(text);
                }
                LayerEvent::Screencopy { request_id, result } => {
                    repaint |= dispatch_screencopy(&mut runtime, None, request_id, result);
                }
                LayerEvent::CaptureOffer {
                    request_id,
                    width,
                    height,
                    device,
                    formats,
                } => {
                    // A lock screen draws one renderer per output, none of
                    // them the one a capture would name: shared memory.
                    repaint |= answer_capture_offer(
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
                LayerEvent::InputMethod(state) => {
                    repaint |= runtime.dispatch_input_method(
                        state.active,
                        state.surrounding_text,
                        state.cursor,
                        state.anchor,
                        state.serial,
                    );
                }
                LayerEvent::TextInput(state) => {
                    repaint |= runtime.dispatch_text_input(
                        state.focused,
                        state.preedit,
                        state.preedit_begin,
                        state.preedit_end,
                        state.commit,
                        state.delete_before,
                        state.delete_after,
                        state.serial,
                    );
                }
                LayerEvent::Key { pressed: false, .. }
                | LayerEvent::Configure { .. }
                | LayerEvent::Scale { .. }
                | LayerEvent::Frame { .. }
                | LayerEvent::PointerMotion { .. }
                | LayerEvent::PointerLeave { .. }
                | LayerEvent::PointerButton { .. }
                | LayerEvent::PointerAxis { .. }
                | LayerEvent::TouchDown { .. }
                | LayerEvent::TouchMotion { .. }
                | LayerEvent::TouchUp { .. }
                | LayerEvent::TouchCancel
                | LayerEvent::PopupConfigure { .. }
                | LayerEvent::PopupFrame { .. }
                | LayerEvent::PopupDone { .. }
                | LayerEvent::FloatingConfigure { .. }
                | LayerEvent::FloatingFrame { .. }
                | LayerEvent::FloatingClose { .. }
                | LayerEvent::Closed { .. } => {}
            }
        }
        apply_service_requests(&mut runtime, &mut client);
        if repaint {
            for (index, renderer) in renderers.iter_mut().enumerate() {
                if let Some(renderer) = renderer {
                    paint_lock(&mut runtime, renderer, &client, index)?;
                }
            }
        }
    }
}

pub(crate) fn paint_lock(
    runtime: &mut Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
    index: usize,
) -> Result<(), String> {
    let (width, height) = client
        .lock_size(index)
        .ok_or_else(|| "lock surface disappeared while painting".to_owned())?;
    let root = primary_surface_root(runtime)?;
    {
        let mut scene = runtime.scene_mut();
        scene
            .assign(root, "x", 0.0)
            .map_err(|error| error.to_string())?;
        scene
            .assign(root, "y", 0.0)
            .map_err(|error| error.to_string())?;
        scene
            .assign(root, "width", width as f64)
            .map_err(|error| error.to_string())?;
        scene
            .assign(root, "height", height as f64)
            .map_err(|error| error.to_string())?;
    }
    let scene = runtime.scene();
    let color = scene
        .color_value(root, "color")
        .map_err(|error| error.to_string())?;
    if color.alpha < 1.0
        || scene
            .number(root, "opacity")
            .map_err(|error| error.to_string())?
            < 1.0
    {
        return Err("lock configuration root must stay opaque".to_owned());
    }
    if scene
        .number(root, "width")
        .map_err(|error| error.to_string())?
        < width as f64
        || scene
            .number(root, "height")
            .map_err(|error| error.to_string())?
            < height as f64
    {
        return Err("lock configuration root must cover the output".to_owned());
    }
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: width as f64,
            height: height as f64,
        },
        renderer.backend_mut(),
    )
    .map_err(|error| error.to_string())?;
    client.request_lock_frame(index);
    let scale = client.lock_scale_120(index).unwrap_or(120);
    let damage = renderer
        .render(&scene, &layout, scale, |_| {})
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        client.commit_lock(index);
    }
    drop(scene);
    runtime.observe_layout(&layout);
    Ok(())
}
