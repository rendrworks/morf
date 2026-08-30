use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mold_io::{IpcIncoming, IpcReply, IpcRequest, IpcServer, IpcValue as WireValue, ipc_call};
use mold_layout::{Layout, ReparentTransition, Size};
use mold_lua::{
    FloatingSurfaceConfig, InputMethodRequest, IpcValue, Limits, PopupSurfaceConfig, Runtime,
    Screen, Screencopy as LuaScreencopy, TextInputRequest, UiEvent, VirtualKeyboardRequest,
    WindowSurfaceAction, WindowSurfaceConfig, WindowSurfaceKind,
};
use mold_render::{RenderEngine, WgpuBackend};
use mold_scene::{Element, NodeHandle};
use mold_wayland::{
    BarConfig, FloatingConfig, FloatingResizeEdge, InputRect, KeyboardFocus, LayerAnchors,
    LayerClient, LayerEvent, OutputPowerMode, PopupAnchor, PopupConfig, PopupConstraints,
    PopupGravity, ScreenInfo, ScreencopyFormat, ShellLayer, SurfaceRole,
};

fn usage() -> &'static str {
    "mold - reactive Wayland shell runtime\n\nusage: mold [--no-plugin | --clean] [shell.lua]\n       mold [--no-plugin | --clean] -c <name>\n       mold lock [lock.lua]\n       mold ipc call <target> [args...]\n       mold ipc verbs\n       mold log [--bindings]\n       mold kill\n       mold --help\n       mold --version"
}

fn run() -> Result<(), String> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    match parse_command(&args)? {
        Command::Help => println!("{}", usage()),
        Command::Version => println!("mold {}", env!("CARGO_PKG_VERSION")),
        Command::Run(path, policy) => {
            let source = fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            supervise(path, source, policy)?;
        }
        Command::Lock(path) => {
            let source = fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            run_lock(&path, &source)?;
        }
        Command::Client(request) => {
            let reply = ipc_call(socket_path()?, &request).map_err(|error| error.to_string())?;
            println!(
                "{}",
                String::from_utf8_lossy(&reply.to_wire().map_err(|e| e.to_string())?)
            );
        }
    }
    Ok(())
}

enum Command {
    Help,
    Version,
    Run(PathBuf, LoadPolicy),
    Lock(PathBuf),
    Client(IpcRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LoadPolicy {
    plugins: bool,
    external_roots: bool,
}

impl Default for LoadPolicy {
    fn default() -> Self {
        Self {
            plugins: true,
            external_roots: true,
        }
    }
}

fn parse_command(args: &[std::ffi::OsString]) -> Result<Command, String> {
    let strings = args
        .iter()
        .map(|value| {
            value
                .to_str()
                .ok_or_else(|| "arguments must be UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (policy, strings) = match strings.as_slice() {
        ["--no-plugin", rest @ ..] => (
            LoadPolicy {
                plugins: false,
                external_roots: true,
            },
            rest,
        ),
        ["--clean", rest @ ..] => (
            LoadPolicy {
                plugins: false,
                external_roots: false,
            },
            rest,
        ),
        strings => (LoadPolicy::default(), strings),
    };
    match strings {
        ["-h" | "--help"] => Ok(Command::Help),
        ["-V" | "--version"] => Ok(Command::Version),
        ["-c", name] => Ok(Command::Run(named_config_path(name)?, policy)),
        ["lock"] => Ok(Command::Lock(config_root()?.join("lock.lua"))),
        ["lock", path] => Ok(Command::Lock(PathBuf::from(path))),
        ["ipc", "verbs"] => Ok(Command::Client(IpcRequest::Verbs)),
        ["ipc", "call", target, args @ ..] => Ok(Command::Client(IpcRequest::Call {
            target: (*target).to_owned(),
            args: args
                .iter()
                .map(|value| WireValue::String((*value).to_owned()))
                .collect(),
        })),
        ["log"] => Ok(Command::Client(IpcRequest::Log)),
        ["log", "--bindings"] => Ok(Command::Client(IpcRequest::Bindings)),
        ["kill"] => Ok(Command::Client(IpcRequest::Kill)),
        [] => Ok(Command::Run(config_root()?.join("shell.lua"), policy)),
        [path] => Ok(Command::Run(PathBuf::from(path), policy)),
        _ => Err(usage().to_owned()),
    }
}

fn config_root() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("mold"));
    }
    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config/mold"))
        .ok_or_else(|| "HOME and XDG_CONFIG_HOME are unset".to_owned())
}

fn named_config_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err("config name must be one path component".to_owned());
    }
    Ok(config_root()?.join(name).join("shell.lua"))
}

fn socket_path() -> Result<PathBuf, String> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is unset".to_owned())?;
    let display = env::var("WAYLAND_DISPLAY").map_err(|_| "WAYLAND_DISPLAY is unset".to_owned())?;
    if display.is_empty() || display.contains('/') {
        return Err("WAYLAND_DISPLAY must be one path component".to_owned());
    }
    Ok(runtime.join("mold").join(format!("{display}.sock")))
}

struct Worker {
    stop: Arc<AtomicBool>,
    commands: mpsc::Sender<WorkerCommand>,
    join: JoinHandle<()>,
    screen: ScreenInfo,
}

enum WorkerCommand {
    Call {
        target: String,
        args: Vec<IpcValue>,
        reply: mpsc::SyncSender<Result<Vec<IpcValue>, String>>,
    },
    Verbs(mpsc::SyncSender<Vec<String>>),
    Logs(mpsc::SyncSender<Vec<String>>),
    Bindings(mpsc::SyncSender<Vec<String>>),
    Reload {
        path: Arc<PathBuf>,
        source: Arc<[u8]>,
        hard: bool,
        reply: mpsc::SyncSender<Result<(), String>>,
    },
}

enum SupervisorMessage {
    Worker(WorkerMessage),
    Ipc(IpcIncoming),
    Reload { hard: bool },
    WatchFiles(bool),
}

enum WorkerMessage {
    Screens {
        output: String,
        screens: Vec<ScreenInfo>,
    },
    Failed {
        output: String,
        error: String,
    },
}

fn run_lock(path: &Path, source: &[u8]) -> Result<(), String> {
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
    apply_output_power_requests(&mut runtime, &mut client);
    apply_screencopy_requests(&mut runtime, &mut client);
    apply_virtual_keyboard_requests(&mut runtime, &mut client);
    apply_input_method_requests(&mut runtime, &mut client);
    apply_text_input_requests(&mut runtime, &mut client);
    let mut renderers: Vec<Option<RenderEngine<WgpuBackend>>> = Vec::new();
    let mut last_frame = None;
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
        apply_output_power_requests(&mut runtime, &mut client);
        apply_clipboard_requests(&mut runtime, &mut client);
        apply_screencopy_requests(&mut runtime, &mut client);
        apply_virtual_keyboard_requests(&mut runtime, &mut client);
        apply_input_method_requests(&mut runtime, &mut client);
        apply_text_input_requests(&mut runtime, &mut client);
        unlock_pending |= runtime.take_session_unlock_request();
        if locked && unlock_pending {
            client.unlock_session().map_err(|error| error.to_string())?;
            return Ok(());
        }
        let next_clock = clock_text();
        if next_clock != clock {
            clock = next_clock;
            runtime
                .update_clock(&clock)
                .map_err(|error| error.to_string())?;
            repaint = true;
        }
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::SessionLocked => locked = true,
                LayerEvent::Screens(_) => {}
                LayerEvent::SessionLockConfigure { index, .. } => {
                    renderers.resize_with(index + 1, || None);
                    let (width, height) = client
                        .lock_physical_size(index)
                        .ok_or_else(|| "configured lock surface disappeared".to_owned())?;
                    if let Some(renderer) = &mut renderers[index] {
                        renderer.backend_mut().resize(width, height);
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
                    let delta = last_frame
                        .map(|previous: u32| time_ms.wrapping_sub(previous).min(250))
                        .unwrap_or(0);
                    last_frame = Some(time_ms);
                    let frame = runtime
                        .tick_animations(Duration::from_millis(delta as u64))
                        .map_err(|error| error.to_string())?;
                    repaint |= frame.active || !frame.changes.is_empty();
                }
                LayerEvent::Key {
                    pressed: true,
                    keysym,
                    text,
                    ..
                } => {
                    if let Some(node) = runtime.first_key_target() {
                        repaint |= runtime.dispatch_key_event(node, keysym, text.as_deref());
                    }
                }
                LayerEvent::SessionLockFinished => {
                    return Err("compositor ended the session lock".to_owned());
                }
                LayerEvent::Idle { timeout_ms, idle } => {
                    repaint |= runtime.dispatch_idle(timeout_ms, idle);
                }
                LayerEvent::OutputPower { .. } => {}
                LayerEvent::Clipboard { text } => {
                    repaint |= runtime.dispatch_clipboard(text);
                }
                LayerEvent::Screencopy { request_id, result } => {
                    repaint |= dispatch_screencopy(&mut runtime, request_id, result);
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
                | LayerEvent::Modifiers { .. }
                | LayerEvent::Configure { .. }
                | LayerEvent::Scale(_)
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
                | LayerEvent::Closed => {}
            }
        }
        apply_clipboard_requests(&mut runtime, &mut client);
        apply_screencopy_requests(&mut runtime, &mut client);
        apply_virtual_keyboard_requests(&mut runtime, &mut client);
        apply_input_method_requests(&mut runtime, &mut client);
        apply_text_input_requests(&mut runtime, &mut client);
        if repaint {
            for (index, renderer) in renderers.iter_mut().enumerate() {
                if let Some(renderer) = renderer {
                    paint_lock(&mut runtime, renderer, &client, index)?;
                }
            }
        }
    }
}

fn paint_lock(
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
        .render(&scene, &layout, scale)
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        client.commit_lock(index);
    }
    drop(scene);
    runtime.observe_layout(&layout);
    Ok(())
}

fn supervise(path: PathBuf, source: Vec<u8>, policy: LoadPolicy) -> Result<(), String> {
    let probe = LayerClient::connect(BarConfig::default()).map_err(|error| error.to_string())?;
    let mut desired = named_screens(probe.screens())?;
    drop(probe);
    if desired.is_empty() {
        return Err("compositor advertised no named outputs".to_owned());
    }
    let path = Arc::new(path);
    let mut source: Arc<[u8]> = source.into();
    let (tx, rx) = mpsc::channel();
    let reload_roots = runtimepath_roots(&path, policy.external_roots);
    let mut reload_snapshot = lua_snapshot(&reload_roots);
    let watch_files = Arc::new(AtomicBool::new(true));
    let watcher_enabled = Arc::clone(&watch_files);
    let reload_tx = tx.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(100));
            let next = lua_snapshot(&reload_roots);
            if !watcher_enabled.load(Ordering::Acquire) {
                reload_snapshot = next;
                continue;
            }
            if next != reload_snapshot {
                reload_snapshot = next;
                if reload_tx
                    .send(SupervisorMessage::Reload { hard: false })
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    let (ipc_tx, ipc_rx) = mpsc::channel();
    let socket = socket_path()?;
    let server = IpcServer::bind(&socket, ipc_tx)
        .map_err(|error| format!("could not bind IPC socket {}: {error}", socket.display()))?;
    let owner = fs::metadata(&socket)
        .map_err(|error| format!("could not inspect IPC socket: {error}"))?
        .uid();
    let forward = tx.clone();
    thread::spawn(move || {
        while let Ok(request) = ipc_rx.recv() {
            if forward.send(SupervisorMessage::Ipc(request)).is_err() {
                break;
            }
        }
    });
    let mut workers = BTreeMap::new();
    let mut daemon_logs = Vec::new();
    reconcile_workers(
        &mut workers,
        &desired,
        Arc::clone(&path),
        Arc::clone(&source),
        policy,
        &tx,
    );

    loop {
        match rx.recv() {
            Ok(SupervisorMessage::Worker(WorkerMessage::Screens { output, screens }))
                if workers.contains_key(&output) =>
            {
                desired = named_screens(&screens)?;
                reconcile_workers(
                    &mut workers,
                    &desired,
                    Arc::clone(&path),
                    Arc::clone(&source),
                    policy,
                    &tx,
                );
            }
            Ok(SupervisorMessage::Worker(WorkerMessage::Screens { .. })) => {}
            Ok(SupervisorMessage::Worker(WorkerMessage::Failed { output, error })) => {
                stop_workers(workers);
                return Err(format!("output {output}: {error}"));
            }
            Ok(SupervisorMessage::Ipc(incoming)) => {
                if incoming.peer.uid != owner {
                    incoming.reply(IpcReply::refused("peer uid does not own the shell"));
                    continue;
                }
                let kill = matches!(incoming.request, IpcRequest::Kill);
                let reply = handle_ipc(&workers, &mut daemon_logs, &incoming.request);
                incoming.reply(reply);
                if kill {
                    stop_workers(workers);
                    drop(server);
                    return Ok(());
                }
            }
            Ok(SupervisorMessage::Reload { hard }) => match fs::read(path.as_ref()) {
                Ok(bytes) => {
                    source = Arc::from(bytes);
                    for (output, worker) in &workers {
                        let (reply, result) = mpsc::sync_channel(1);
                        if worker
                            .commands
                            .send(WorkerCommand::Reload {
                                path: Arc::clone(&path),
                                source: Arc::clone(&source),
                                hard,
                                reply,
                            })
                            .is_err()
                        {
                            daemon_logs.push(format!("reload {output}: output stopped"));
                            continue;
                        }
                        match result.recv_timeout(Duration::from_secs(2)) {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => daemon_logs.push(format!("reload {output}: {error}")),
                            Err(_) => daemon_logs.push(format!("reload {output}: timed out")),
                        }
                    }
                }
                Err(error) => daemon_logs.push(format!("reload: {error}")),
            },
            Ok(SupervisorMessage::WatchFiles(enabled)) => {
                watch_files.store(enabled, Ordering::Release);
            }
            Err(_) => return Err("all output workers stopped".to_owned()),
        }
    }
}

fn named_screens(screens: &[ScreenInfo]) -> Result<BTreeMap<String, ScreenInfo>, String> {
    screens
        .iter()
        .map(|screen| {
            screen
                .name
                .clone()
                .map(|name| (name, screen.clone()))
                .ok_or_else(|| format!("output {} has no compositor name", screen.id))
        })
        .collect()
}

fn runtimepath_roots(config: &Path, external: bool) -> Vec<PathBuf> {
    let mut roots = config
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect::<Vec<_>>();
    if external {
        roots.extend(
            env::var_os("MOLD_RUNTIME_PATH")
                .into_iter()
                .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>()),
        );
        if let Some(data) = env::var_os("XDG_DATA_HOME") {
            roots.push(PathBuf::from(data).join("mold/site"));
        } else if let Some(home) = env::var_os("HOME") {
            roots.push(PathBuf::from(home).join(".local/share/mold/site"));
        }
    }
    let mut unique = Vec::new();
    for root in roots {
        if !unique.contains(&root) {
            unique.push(root);
        }
    }
    unique
}

fn execute_config(
    runtime: &mut Runtime,
    path: &Path,
    source: &[u8],
    policy: LoadPolicy,
) -> Result<(), String> {
    let roots = runtimepath_roots(path, policy.external_roots);
    runtime.set_module_roots(roots.clone());
    runtime.set_shell_root(
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    );
    for plugin in policy
        .plugins
        .then(|| runtime_scripts(&roots, "plugin"))
        .into_iter()
        .flatten()
    {
        match fs::read(&plugin) {
            Ok(source) => {
                if let Err(error) = runtime.execute(&plugin.to_string_lossy(), &source) {
                    eprintln!("mold: plugin {}: {error}", plugin.display());
                }
            }
            Err(error) => eprintln!("mold: plugin {}: {error}", plugin.display()),
        }
    }
    runtime
        .execute(&path.to_string_lossy(), source)
        .map_err(|error| error.to_string())?;
    for after in policy
        .plugins
        .then(|| runtime_scripts(&roots, "after/plugin"))
        .into_iter()
        .flatten()
    {
        match fs::read(&after) {
            Ok(source) => {
                if let Err(error) = runtime.execute(&after.to_string_lossy(), &source) {
                    eprintln!("mold: after plugin {}: {error}", after.display());
                }
            }
            Err(error) => eprintln!("mold: after plugin {}: {error}", after.display()),
        }
    }
    Ok(())
}

fn runtime_scripts(roots: &[PathBuf], directory: &str) -> Vec<PathBuf> {
    let mut scripts = Vec::new();
    for root in roots {
        let mut found = Vec::new();
        collect_lua_scripts(&root.join(directory), &mut found);
        found.sort();
        for path in found {
            if !scripts.contains(&path) {
                scripts.push(path);
            }
        }
    }
    scripts
}

fn collect_lua_scripts(path: &Path, scripts: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lua_scripts(&path, scripts);
        } else if path.extension().and_then(|value| value.to_str()) == Some("lua") {
            scripts.push(path);
        }
    }
}

fn lua_snapshot(roots: &[PathBuf]) -> BTreeMap<PathBuf, (u64, SystemTime)> {
    let mut snapshot = BTreeMap::new();
    let mut pending = roots.to_vec();
    while let Some(path) = pending.pop() {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("lua") {
                continue;
            }
            if let Ok(metadata) = entry.metadata() {
                snapshot.insert(
                    path,
                    (
                        metadata.len(),
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    ),
                );
            }
        }
    }
    snapshot
}

fn reconcile_workers(
    workers: &mut BTreeMap<String, Worker>,
    desired: &BTreeMap<String, ScreenInfo>,
    path: Arc<PathBuf>,
    source: Arc<[u8]>,
    policy: LoadPolicy,
    tx: &mpsc::Sender<SupervisorMessage>,
) {
    let stale = workers
        .iter()
        .filter(|(name, worker)| desired.get(*name) != Some(&worker.screen))
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    for name in stale {
        let worker = workers.remove(&name).expect("worker key is present");
        worker.stop.store(true, Ordering::Release);
        let _ = worker.join.join();
    }
    for (name, screen) in desired {
        if workers.contains_key(name) {
            continue;
        }
        let output = name.clone();
        let screen = screen.clone();
        let worker_screen = screen.clone();
        let path = Arc::clone(&path);
        let source = Arc::clone(&source);
        let tx = tx.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (commands, command_rx) = mpsc::channel();
        let join = thread::spawn(move || {
            if let Err(error) = run_surface(
                &path,
                &source,
                screen,
                policy,
                &tx,
                &worker_stop,
                &command_rx,
            ) && !worker_stop.load(Ordering::Acquire)
            {
                let _ = tx.send(SupervisorMessage::Worker(WorkerMessage::Failed {
                    output,
                    error,
                }));
            }
        });
        workers.insert(
            name.clone(),
            Worker {
                stop,
                commands,
                join,
                screen: worker_screen,
            },
        );
    }
}

fn handle_ipc(
    workers: &BTreeMap<String, Worker>,
    daemon_logs: &mut Vec<String>,
    request: &IpcRequest,
) -> IpcReply {
    match request {
        IpcRequest::Call { target, args } => {
            let Some(worker) = workers.values().next() else {
                return IpcReply::refused("shell has no active output");
            };
            let args = args.iter().map(lua_ipc_value).collect::<Vec<_>>();
            let (tx, rx) = mpsc::sync_channel(1);
            if worker
                .commands
                .send(WorkerCommand::Call {
                    target: target.clone(),
                    args,
                    reply: tx,
                })
                .is_err()
            {
                return IpcReply::refused("shell output stopped");
            }
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(values)) => IpcReply::success(values.iter().map(wire_ipc_value).collect()),
                Ok(Err(error)) => IpcReply::refused(error),
                Err(_) => IpcReply::refused("shell output timed out"),
            }
        }
        IpcRequest::Verbs => {
            let mut verbs = Vec::new();
            for worker in workers.values() {
                let (tx, rx) = mpsc::sync_channel(1);
                if worker.commands.send(WorkerCommand::Verbs(tx)).is_ok()
                    && let Ok(found) = rx.recv_timeout(Duration::from_secs(1))
                {
                    verbs.extend(found);
                }
            }
            verbs.sort();
            verbs.dedup();
            IpcReply::success(verbs.into_iter().map(WireValue::String).collect())
        }
        IpcRequest::Log => {
            let mut logs = std::mem::take(daemon_logs);
            for worker in workers.values() {
                let (tx, rx) = mpsc::sync_channel(1);
                if worker.commands.send(WorkerCommand::Logs(tx)).is_ok()
                    && let Ok(found) = rx.recv_timeout(Duration::from_secs(1))
                {
                    logs.extend(found);
                }
            }
            IpcReply::success(logs.into_iter().map(WireValue::String).collect())
        }
        IpcRequest::Bindings => {
            let mut bindings = Vec::new();
            for worker in workers.values() {
                let (tx, rx) = mpsc::sync_channel(1);
                if worker.commands.send(WorkerCommand::Bindings(tx)).is_ok()
                    && let Ok(found) = rx.recv_timeout(Duration::from_secs(1))
                {
                    bindings.extend(found);
                }
            }
            bindings.sort();
            bindings.dedup();
            IpcReply::success(bindings.into_iter().map(WireValue::String).collect())
        }
        IpcRequest::Kill => IpcReply::success(Vec::new()),
    }
}

#[derive(Clone, Copy, Default)]
struct WorkerUpdate {
    repaint: bool,
    reset_input: bool,
    refresh_idle: bool,
    recreate_surface: bool,
}

fn handle_worker_command(
    runtime: &mut Runtime,
    screen: &Screen,
    policy: LoadPolicy,
    command: WorkerCommand,
) -> WorkerUpdate {
    match command {
        WorkerCommand::Call {
            target,
            args,
            reply,
        } => {
            let result = runtime
                .call_ipc(&target, &args)
                .map_err(|error| error.to_string());
            let repaint = result.is_ok();
            let _ = reply.send(result);
            WorkerUpdate {
                repaint,
                reset_input: false,
                refresh_idle: false,
                recreate_surface: false,
            }
        }
        WorkerCommand::Verbs(reply) => {
            let _ = reply.send(runtime.ipc_verbs());
            WorkerUpdate::default()
        }
        WorkerCommand::Logs(reply) => {
            let _ = reply.send(runtime.take_logs());
            WorkerUpdate::default()
        }
        WorkerCommand::Bindings(reply) => {
            let _ = reply.send(runtime.binding_dependencies());
            WorkerUpdate::default()
        }
        WorkerCommand::Reload {
            path,
            source,
            hard,
            reply,
        } => {
            let mut candidate = Runtime::for_screen(Limits::default(), screen.clone());
            if !hard {
                candidate.restore_reloadable_state(runtime.reloadable_state());
            }
            let result = execute_config(&mut candidate, &path, &source, policy)
                .and_then(|()| primary_surface_root(&candidate).map(|_| ()))
                .and_then(|()| {
                    candidate
                        .update_clock(clock_text())
                        .map_err(|error| error.to_string())
                });
            let repaint = match &result {
                Ok(()) => {
                    candidate.dispatch_reload_completed();
                    *runtime = candidate;
                    true
                }
                Err(error) => {
                    runtime.dispatch_reload_failed(error.clone());
                    false
                }
            };
            let _ = reply.send(result);
            WorkerUpdate {
                repaint,
                reset_input: repaint,
                refresh_idle: repaint,
                recreate_surface: repaint && hard,
            }
        }
    }
}

fn lua_ipc_value(value: &WireValue) -> IpcValue {
    match value {
        WireValue::Nil => IpcValue::Nil,
        WireValue::Boolean(value) => IpcValue::Boolean(*value),
        WireValue::Integer(value) => IpcValue::Integer(*value),
        WireValue::Number(value) => IpcValue::Number(*value),
        WireValue::String(value) => IpcValue::String(value.clone()),
    }
}

fn wire_ipc_value(value: &IpcValue) -> WireValue {
    match value {
        IpcValue::Nil => WireValue::Nil,
        IpcValue::Boolean(value) => WireValue::Boolean(*value),
        IpcValue::Integer(value) => WireValue::Integer(*value),
        IpcValue::Number(value) => WireValue::Number(*value),
        IpcValue::String(value) => WireValue::String(value.clone()),
    }
}

fn apply_output_power_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for on in runtime.take_output_power_requests() {
        client.set_output_power(if on {
            OutputPowerMode::On
        } else {
            OutputPowerMode::Off
        });
    }
}

fn apply_clipboard_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    if !client.can_set_clipboard() {
        return;
    }
    for text in runtime.take_clipboard_requests() {
        client.set_clipboard(text);
    }
}

fn apply_screencopy_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for request in runtime.take_screencopy_requests() {
        if !client.capture_output(request.id, request.include_cursor) {
            runtime.dispatch_screencopy(request.id, Err("screencopy is unavailable".to_owned()));
        }
    }
}

fn dispatch_screencopy(
    runtime: &mut Runtime,
    request_id: u64,
    result: Result<mold_wayland::ScreencopyFrame, String>,
) -> bool {
    runtime.dispatch_screencopy(
        request_id,
        result.map(|frame| LuaScreencopy {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            format: match frame.format {
                ScreencopyFormat::Argb8888 => "argb8888",
                ScreencopyFormat::Xrgb8888 => "xrgb8888",
            }
            .to_owned(),
            y_invert: frame.y_invert,
            pixels: frame.pixels,
        }),
    )
}

fn apply_virtual_keyboard_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for request in runtime.take_virtual_keyboard_requests() {
        match request {
            VirtualKeyboardRequest::Key { keycode, pressed } => {
                client.send_virtual_key(keycode, pressed);
            }
            VirtualKeyboardRequest::Modifiers {
                depressed,
                latched,
                locked,
                group,
            } => {
                client.send_virtual_modifiers(depressed, latched, locked, group);
            }
        }
    }
}

fn apply_input_method_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    if runtime.take_input_method_enable_request() {
        client.enable_input_method();
    }
    for request in runtime.take_input_method_requests() {
        match request {
            InputMethodRequest::Commit(text) => {
                client.input_method_commit(&text);
            }
            InputMethodRequest::Preedit { text, begin, end } => {
                client.input_method_preedit(&text, begin, end);
            }
            InputMethodRequest::Delete { before, after } => {
                client.input_method_delete(before, after);
            }
        }
    }
}

fn apply_text_input_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    if runtime.take_text_input_enable_request() {
        client.enable_text_input();
    }
    for request in runtime.take_text_input_requests() {
        match request {
            TextInputRequest::Disable => {
                client.disable_text_input();
            }
            TextInputRequest::Surrounding {
                text,
                cursor,
                anchor,
            } => {
                client.set_text_input_surrounding(&text, cursor, anchor);
            }
            TextInputRequest::ContentType { hints, purpose } => {
                client.set_text_input_content_type(hints, purpose);
            }
            TextInputRequest::CursorRect {
                x,
                y,
                width,
                height,
            } => {
                client.set_text_input_cursor_rect(InputRect {
                    x,
                    y,
                    width,
                    height,
                });
            }
        }
    }
}

fn stop_workers(workers: BTreeMap<String, Worker>) {
    for worker in workers.values() {
        worker.stop.store(true, Ordering::Release);
    }
    for (_, worker) in workers {
        let _ = worker.join.join();
    }
}

struct AuxiliarySurface {
    id: u64,
    root: NodeHandle,
    width: u32,
    height: u32,
    renderer: Option<RenderEngine<WgpuBackend>>,
    layout: Option<Layout>,
    popup_config: Option<PopupSurfaceConfig>,
    floating_config: Option<FloatingSurfaceConfig>,
}

fn popup_anchor(value: &str) -> Result<PopupAnchor, String> {
    Ok(match value {
        "none" => PopupAnchor::None,
        "top" => PopupAnchor::Top,
        "bottom" => PopupAnchor::Bottom,
        "left" => PopupAnchor::Left,
        "right" => PopupAnchor::Right,
        "top_left" => PopupAnchor::TopLeft,
        "top_right" => PopupAnchor::TopRight,
        "bottom_left" => PopupAnchor::BottomLeft,
        "bottom_right" => PopupAnchor::BottomRight,
        value => return Err(format!("invalid popup anchor `{value}`")),
    })
}

fn popup_gravity(value: &str) -> Result<PopupGravity, String> {
    Ok(match value {
        "none" => PopupGravity::None,
        "top" => PopupGravity::Top,
        "bottom" => PopupGravity::Bottom,
        "left" => PopupGravity::Left,
        "right" => PopupGravity::Right,
        "top_left" => PopupGravity::TopLeft,
        "top_right" => PopupGravity::TopRight,
        "bottom_left" => PopupGravity::BottomLeft,
        "bottom_right" => PopupGravity::BottomRight,
        value => return Err(format!("invalid popup gravity `{value}`")),
    })
}

fn window_surface_parent(surface: &WindowSurfaceConfig) -> Option<u64> {
    match &surface.kind {
        WindowSurfaceKind::Popup(config) => config.parent,
        WindowSurfaceKind::Floating(config) => config.parent,
    }
}

fn window_surface_effectively_visible(
    id: u64,
    surfaces: &HashMap<u64, &WindowSurfaceConfig>,
    visiting: &mut HashSet<u64>,
) -> bool {
    let Some(surface) = surfaces.get(&id) else {
        return false;
    };
    if !surface.visible || !visiting.insert(id) {
        return false;
    }
    let visible = window_surface_parent(surface)
        .is_none_or(|parent| window_surface_effectively_visible(parent, surfaces, visiting));
    visiting.remove(&id);
    visible
}

fn sync_window_surfaces(
    runtime: &Runtime,
    client: &mut LayerClient,
    popups: &mut HashMap<u64, AuxiliarySurface>,
    floatings: &mut HashMap<u64, AuxiliarySurface>,
) -> Result<(), String> {
    let surfaces = runtime.window_surface_configs();
    let surfaces_by_id = surfaces
        .iter()
        .map(|surface| (surface.id, surface))
        .collect::<HashMap<_, _>>();
    let mut desired_popups = surfaces
        .iter()
        .filter(|surface| {
            matches!(&surface.kind, WindowSurfaceKind::Popup(_))
                && window_surface_effectively_visible(
                    surface.id,
                    &surfaces_by_id,
                    &mut HashSet::new(),
                )
        })
        .collect::<Vec<_>>();
    let mut desired_floatings = surfaces
        .iter()
        .filter(|surface| {
            matches!(&surface.kind, WindowSurfaceKind::Floating(_))
                && window_surface_effectively_visible(
                    surface.id,
                    &surfaces_by_id,
                    &mut HashSet::new(),
                )
        })
        .collect::<Vec<_>>();
    desired_popups.sort_by_key(|surface| surface.id);
    desired_floatings.sort_by_key(|surface| surface.id);
    let desired_popup_ids = desired_popups
        .iter()
        .map(|surface| surface.id)
        .collect::<HashSet<_>>();
    let desired_floating_ids = desired_floatings
        .iter()
        .map(|surface| surface.id)
        .collect::<HashSet<_>>();

    let mut stale_popups = popups
        .keys()
        .filter(|id| !desired_popup_ids.contains(id))
        .copied()
        .collect::<Vec<_>>();
    stale_popups.sort_unstable_by(|a, b| b.cmp(a));
    for id in stale_popups {
        client.close_popup(id);
        popups.remove(&id);
    }
    let mut stale_floatings = floatings
        .keys()
        .filter(|id| !desired_floating_ids.contains(id))
        .copied()
        .collect::<Vec<_>>();
    stale_floatings.sort_unstable_by(|a, b| b.cmp(a));
    for id in stale_floatings {
        client.close_floating(id);
        floatings.remove(&id);
    }
    let mut reopened = HashSet::new();
    for surface in desired_floatings {
        let id = surface.id;
        let WindowSurfaceKind::Floating(config) = &surface.kind else {
            unreachable!();
        };
        let changed = floatings
            .get(&id)
            .is_none_or(|current| current.floating_config.as_ref() != Some(config))
            || config
                .parent
                .is_some_and(|parent| reopened.contains(&parent));
        if changed {
            client.close_floating(id);
            client
                .open_floating(
                    id,
                    config.parent,
                    FloatingConfig {
                        width: config.width,
                        height: config.height,
                        minimum_width: config.minimum_width,
                        minimum_height: config.minimum_height,
                        maximum_width: config.maximum_width,
                        maximum_height: config.maximum_height,
                        title: config.title.clone(),
                        app_id: config.app_id.clone(),
                        minimized: config.minimized,
                        maximized: config.maximized,
                        fullscreen: config.fullscreen,
                    },
                )
                .map_err(|error| error.to_string())?;
            reopened.insert(id);
            floatings.insert(
                id,
                AuxiliarySurface {
                    id: surface.id,
                    root: surface.root,
                    width: config.width,
                    height: config.height,
                    renderer: None,
                    layout: None,
                    popup_config: None,
                    floating_config: Some(config.clone()),
                },
            );
        } else if let Some(current) = floatings.get_mut(&id) {
            current.root = surface.root;
            current.width = config.width;
            current.height = config.height;
            current.layout = None;
        }
    }
    for surface in desired_popups {
        let id = surface.id;
        let WindowSurfaceKind::Popup(config) = &surface.kind else {
            unreachable!();
        };
        let changed = popups
            .get(&id)
            .is_none_or(|current| current.popup_config.as_ref() != Some(config))
            || config
                .parent
                .is_some_and(|parent| reopened.contains(&parent));
        if changed {
            client.close_popup(id);
            let parent = if let Some(parent) = config.parent {
                let parent = surfaces_by_id
                    .get(&parent)
                    .ok_or_else(|| "popup parent is stale".to_owned())?;
                match parent.kind {
                    WindowSurfaceKind::Popup(_) => SurfaceRole::Popup(parent.id),
                    WindowSurfaceKind::Floating(_) => SurfaceRole::Floating(parent.id),
                }
            } else {
                SurfaceRole::Layer
            };
            client
                .open_popup(
                    id,
                    parent,
                    PopupConfig {
                        anchor: InputRect {
                            x: config.anchor_x,
                            y: config.anchor_y,
                            width: config.anchor_width,
                            height: config.anchor_height,
                        },
                        width: config.width,
                        height: config.height,
                        anchor_edge: popup_anchor(&config.anchor_edge)?,
                        gravity: popup_gravity(&config.gravity)?,
                        offset_x: config.offset_x,
                        offset_y: config.offset_y,
                        constraints: PopupConstraints {
                            slide_x: config.constraints.slide_x,
                            slide_y: config.constraints.slide_y,
                            flip_x: config.constraints.flip_x,
                            flip_y: config.constraints.flip_y,
                            resize_x: config.constraints.resize_x,
                            resize_y: config.constraints.resize_y,
                        },
                        grab_focus: config.grab_focus,
                    },
                )
                .map_err(|error| error.to_string())?;
            reopened.insert(id);
            popups.insert(
                id,
                AuxiliarySurface {
                    id: surface.id,
                    root: surface.root,
                    width: config.width,
                    height: config.height,
                    renderer: None,
                    layout: None,
                    popup_config: Some(config.clone()),
                    floating_config: None,
                },
            );
        } else if let Some(current) = popups.get_mut(&id) {
            current.root = surface.root;
            current.width = config.width;
            current.height = config.height;
            current.layout = None;
        }
    }
    Ok(())
}

fn auxiliary_physical_size(width: u32, height: u32, scale_120: u32) -> (u32, u32) {
    let physical = |value: u32| {
        u32::try_from((u64::from(value) * u64::from(scale_120)).div_ceil(120)).unwrap_or(u32::MAX)
    };
    (physical(width), physical(height))
}

fn surface_layout<'a>(
    surface: SurfaceRole,
    layer: &'a Layout,
    popups: &'a HashMap<u64, AuxiliarySurface>,
    floatings: &'a HashMap<u64, AuxiliarySurface>,
) -> Option<&'a Layout> {
    match surface {
        SurfaceRole::Layer => Some(layer),
        SurfaceRole::Popup(id) => popups.get(&id)?.layout.as_ref(),
        SurfaceRole::Floating(id) => floatings.get(&id)?.layout.as_ref(),
    }
}

fn surface_root(
    surface: SurfaceRole,
    layer: NodeHandle,
    popups: &HashMap<u64, AuxiliarySurface>,
    floatings: &HashMap<u64, AuxiliarySurface>,
) -> Option<NodeHandle> {
    match surface {
        SurfaceRole::Layer => Some(layer),
        SurfaceRole::Popup(id) => popups.get(&id).map(|surface| surface.root),
        SurfaceRole::Floating(id) => floatings.get(&id).map(|surface| surface.root),
    }
}

fn primary_surface_root(runtime: &Runtime) -> Result<NodeHandle, String> {
    let roots = runtime.scene().roots();
    let mut window_roots = HashSet::new();
    for surface in runtime.window_surface_configs() {
        if !window_roots.insert(surface.root) {
            return Err("a scene root cannot back multiple window surfaces".into());
        }
        if runtime
            .scene()
            .parent(surface.root)
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("window surface roots must be top-level scene nodes".into());
        }
    }
    let primary = roots
        .into_iter()
        .filter(|root| !window_roots.contains(root))
        .collect::<Vec<_>>();
    if primary.len() != 1 {
        return Err("configuration must create exactly one primary surface root".into());
    }
    Ok(primary[0])
}

fn runtime_bar_config(runtime: &Runtime, output: &str) -> Result<BarConfig, String> {
    let surface = runtime.layer_surface_config();
    let layer = match surface.layer.as_str() {
        "background" => ShellLayer::Background,
        "bottom" => ShellLayer::Bottom,
        "top" => ShellLayer::Top,
        "overlay" => ShellLayer::Overlay,
        value => return Err(format!("unsupported layer surface layer `{value}`")),
    };
    let keyboard_focus = match surface.keyboard_focus.as_str() {
        "none" => KeyboardFocus::None,
        "exclusive" => KeyboardFocus::Exclusive,
        "on_demand" => KeyboardFocus::OnDemand,
        value => return Err(format!("unsupported keyboard focus policy `{value}`")),
    };
    Ok(BarConfig {
        namespace: surface.namespace,
        width: surface.width,
        height: surface.height,
        exclusive_zone: surface.exclusive_zone,
        output: Some(output.to_owned()),
        anchors: LayerAnchors {
            top: surface.anchors.top,
            right: surface.anchors.right,
            bottom: surface.anchors.bottom,
            left: surface.anchors.left,
        },
        margin_top: surface.margin_top,
        margin_right: surface.margin_right,
        margin_bottom: surface.margin_bottom,
        margin_left: surface.margin_left,
        layer,
        keyboard_focus,
    })
}

fn connect_runtime_surface(runtime: &Runtime, output: &str) -> Result<LayerClient, String> {
    let mut client = LayerClient::connect(runtime_bar_config(runtime, output)?)
        .map_err(|error| error.to_string())?;
    loop {
        client.dispatch().map_err(|error| error.to_string())?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::Configure { .. } => return Ok(client),
                LayerEvent::Closed => return Err("layer surface was closed".to_owned()),
                _ => {}
            }
        }
    }
}

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

    let mut client = LayerClient::connect(runtime_bar_config(&runtime, &name)?)
        .map_err(|error| error.to_string())?;

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
                LayerEvent::Configure { .. } => break 'configured,
                LayerEvent::Closed => return Err("layer surface was closed".to_owned()),
                LayerEvent::Screencopy { request_id, result } => {
                    dispatch_screencopy(&mut runtime, request_id, result);
                }
                LayerEvent::Scale(_)
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
    let mut layout = paint(&runtime, &mut renderer, &client)?;
    let mut popup_surfaces = HashMap::new();
    let mut floating_surfaces = HashMap::new();
    runtime.take_window_surface_change();
    sync_window_surfaces(
        &runtime,
        &mut client,
        &mut popup_surfaces,
        &mut floating_surfaces,
    )?;
    apply_output_power_requests(&mut runtime, &mut client);
    apply_screencopy_requests(&mut runtime, &mut client);
    apply_virtual_keyboard_requests(&mut runtime, &mut client);
    apply_input_method_requests(&mut runtime, &mut client);
    apply_text_input_requests(&mut runtime, &mut client);

    let mut last_frame = None;
    let mut hovered = None::<(SurfaceRole, NodeHandle)>;
    let mut pressed = None::<(SurfaceRole, NodeHandle, f64, f64, bool)>;
    let mut focused = HashMap::<SurfaceRole, NodeHandle>::new();
    let mut touches = HashMap::<i32, (SurfaceRole, NodeHandle, f64, f64)>::new();
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
                hovered = None;
                pressed = None;
                focused.clear();
                touches.clear();
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
            popup_surfaces.clear();
            floating_surfaces.clear();
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
            sync_window_surfaces(
                &runtime,
                &mut client,
                &mut popup_surfaces,
                &mut floating_surfaces,
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
            match event {
                LayerEvent::Configure { .. } | LayerEvent::Scale(_) => {
                    let (width, height) = client.physical_size();
                    renderer.backend_mut().resize(width, height);
                    for surface in popup_surfaces
                        .values_mut()
                        .chain(floating_surfaces.values_mut())
                    {
                        if let Some(renderer) = &mut surface.renderer {
                            let (width, height) = auxiliary_physical_size(
                                surface.width,
                                surface.height,
                                client.scale_120(),
                            );
                            renderer.backend_mut().resize(width, height);
                        }
                    }
                    repaint = true;
                }
                LayerEvent::Frame { time_ms } => {
                    let delta = last_frame
                        .map(|previous: u32| time_ms.wrapping_sub(previous).min(250))
                        .unwrap_or(0);
                    last_frame = Some(time_ms);
                    let frame = runtime
                        .tick_animations(Duration::from_millis(delta as u64))
                        .map_err(|error| error.to_string())?;
                    repaint |= frame.active || !frame.changes.is_empty();
                }
                LayerEvent::Closed => return Err("layer surface was closed".to_owned()),
                LayerEvent::Idle { timeout_ms, idle } => {
                    repaint |= runtime.dispatch_idle(timeout_ms, idle);
                }
                LayerEvent::OutputPower { .. } => {}
                LayerEvent::Clipboard { text } => {
                    repaint |= runtime.dispatch_clipboard(text);
                }
                LayerEvent::Screencopy { request_id, result } => {
                    repaint |= dispatch_screencopy(&mut runtime, request_id, result);
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
                LayerEvent::PointerMotion { surface, x, y } => {
                    let Some(hit_layout) =
                        surface_layout(surface, &layout, &popup_surfaces, &floating_surfaces)
                    else {
                        continue;
                    };
                    let hit = hit_layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    let next_hovered = hit.map(|node| (surface, node));
                    if next_hovered != hovered {
                        if let Some((_, node)) = hovered {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerExited);
                        }
                        if let Some(node) = hit {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerEntered);
                        }
                        hovered = next_hovered;
                    }
                    if let Some(node) = hit {
                        repaint |= runtime.dispatch_pointer_event(
                            node,
                            UiEvent::PointerMoved,
                            x,
                            y,
                            0.0,
                            0.0,
                        );
                    }
                    if let Some((pressed_surface, node, start_x, start_y, dragging)) = &mut pressed
                        && *pressed_surface == surface
                    {
                        let delta_x = x - *start_x;
                        let delta_y = y - *start_y;
                        if !*dragging && delta_x.hypot(delta_y) >= 8.0 {
                            *dragging = true;
                            repaint |= runtime.dispatch_pointer_event(
                                *node,
                                UiEvent::DragStarted,
                                x,
                                y,
                                delta_x,
                                delta_y,
                            );
                        }
                        if *dragging {
                            repaint |= runtime.dispatch_pointer_event(
                                *node,
                                UiEvent::Dragged,
                                x,
                                y,
                                delta_x,
                                delta_y,
                            );
                        }
                    }
                }
                LayerEvent::PointerLeave { surface } => {
                    if hovered.is_some_and(|(hovered_surface, _)| hovered_surface == surface)
                        && let Some((_, node)) = hovered.take()
                    {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerExited);
                    }
                }
                LayerEvent::PointerAxis {
                    surface,
                    x,
                    y,
                    horizontal,
                    vertical,
                    horizontal_steps,
                    vertical_steps,
                } => {
                    let Some(hit_layout) =
                        surface_layout(surface, &layout, &popup_surfaces, &floating_surfaces)
                    else {
                        continue;
                    };
                    let hit = hit_layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    if let Some(node) = hit {
                        repaint |= runtime.dispatch_wheel_event(
                            node,
                            (x, y),
                            (horizontal, vertical),
                            (horizontal_steps, vertical_steps),
                        );
                    }
                }
                LayerEvent::PointerButton {
                    surface,
                    button,
                    pressed: true,
                    x,
                    y,
                } => {
                    let Some(hit_layout) =
                        surface_layout(surface, &layout, &popup_surfaces, &floating_surfaces)
                    else {
                        continue;
                    };
                    let hit = hit_layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    let hit = hit.filter(|node| runtime.accepts_pointer_button(*node, button));
                    pressed = hit.map(|node| (surface, node, x, y, false));
                    if let Some(target) = hit.and_then(|node| runtime.key_target_for_node(node)) {
                        focused.insert(surface, target);
                    } else {
                        focused.remove(&surface);
                    }
                    if let Some(node) = hit {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Pressed);
                    }
                }
                LayerEvent::TouchDown { surface, id, x, y } => {
                    let Some(hit_layout) =
                        surface_layout(surface, &layout, &popup_surfaces, &floating_surfaces)
                    else {
                        continue;
                    };
                    let hit = hit_layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    if let Some(node) = hit {
                        touches.insert(id, (surface, node, x, y));
                        if let Some(target) = runtime.key_target_for_node(node) {
                            focused.insert(surface, target);
                        } else {
                            focused.remove(&surface);
                        }
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Pressed);
                        repaint |=
                            runtime.dispatch_touch_event(node, UiEvent::TouchPressed, id, x, y);
                    }
                }
                LayerEvent::TouchMotion { id, x, y, .. } => {
                    if let Some((_, node, last_x, last_y)) = touches.get_mut(&id) {
                        *last_x = x;
                        *last_y = y;
                        repaint |=
                            runtime.dispatch_touch_event(*node, UiEvent::TouchMoved, id, x, y);
                    }
                }
                LayerEvent::TouchUp { surface, id, x, y } => {
                    if let Some((touch_surface, node, _, _)) = touches.remove(&id) {
                        repaint |=
                            runtime.dispatch_touch_event(node, UiEvent::TouchReleased, id, x, y);
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
                        let hit =
                            surface_layout(surface, &layout, &popup_surfaces, &floating_surfaces)
                                .filter(|_| touch_surface == surface)
                                .map(|layout| layout.hit_test(&runtime.scene(), x, y))
                                .transpose()
                                .map_err(|error| error.to_string())?
                                .flatten();
                        if hit == Some(node) {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::Clicked);
                        }
                    }
                }
                LayerEvent::TouchCancel => {
                    for (id, (_, node, x, y)) in touches.drain() {
                        repaint |=
                            runtime.dispatch_touch_event(node, UiEvent::TouchCanceled, id, x, y);
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
                    }
                }
                LayerEvent::PointerButton {
                    surface,
                    pressed: false,
                    x,
                    y,
                    ..
                } => {
                    let hit = surface_layout(surface, &layout, &popup_surfaces, &floating_surfaces)
                        .map(|layout| layout.hit_test(&runtime.scene(), x, y))
                        .transpose()
                        .map_err(|error| error.to_string())?
                        .flatten();
                    if let Some((pressed_surface, node, start_x, start_y, dragging)) =
                        pressed.take()
                    {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
                        if dragging {
                            repaint |= runtime.dispatch_pointer_event(
                                node,
                                UiEvent::DragFinished,
                                x,
                                y,
                                x - start_x,
                                y - start_y,
                            );
                        } else if pressed_surface == surface && hit == Some(node) {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::Clicked);
                        }
                    }
                }
                LayerEvent::Key {
                    surface,
                    pressed: true,
                    keysym,
                    text,
                    ..
                } => {
                    let Some(root) = surface_root(
                        surface,
                        primary_surface_root(&runtime)?,
                        &popup_surfaces,
                        &floating_surfaces,
                    ) else {
                        continue;
                    };
                    let current = focused
                        .get(&surface)
                        .copied()
                        .filter(|node| runtime.node_in_subtree(root, *node));
                    if keysym == 0xff09 {
                        if let Some(next) = runtime.next_key_target_in(root, current) {
                            focused.insert(surface, next);
                        } else {
                            focused.remove(&surface);
                        }
                        repaint = true;
                    } else if let Some(node) = current.or_else(|| runtime.first_key_target_in(root))
                    {
                        focused.insert(surface, node);
                        repaint |= runtime.dispatch_key_event(node, keysym, text.as_deref());
                    }
                }
                LayerEvent::PopupConfigure { id, width, height } => {
                    if let Some(surface) = popup_surfaces.get_mut(&id) {
                        surface.width = width.max(1);
                        surface.height = height.max(1);
                        let (physical_width, physical_height) = auxiliary_physical_size(
                            surface.width,
                            surface.height,
                            client.scale_120(),
                        );
                        if let Some(renderer) = &mut surface.renderer {
                            renderer
                                .backend_mut()
                                .resize(physical_width, physical_height);
                        } else {
                            let target = client
                                .popup_window_target(id)
                                .ok_or_else(|| "configured popup disappeared".to_owned())?;
                            let backend = pollster::block_on(WgpuBackend::new_surface(
                                target,
                                physical_width,
                                physical_height,
                            ))
                            .map_err(|error| error.to_string())?;
                            surface.renderer = Some(RenderEngine::new(backend));
                        }
                        paint_popup_surface(&runtime, &client, surface)?;
                    }
                }
                LayerEvent::PopupFrame { id, .. } => {
                    if let Some(surface) = popup_surfaces.get_mut(&id) {
                        paint_popup_surface(&runtime, &client, surface)?;
                    }
                }
                LayerEvent::PopupDone { id } => {
                    if let Some(surface) = popup_surfaces.remove(&id) {
                        runtime.set_window_surface_visible(surface.id, false);
                    }
                }
                LayerEvent::FloatingConfigure { id, width, height } => {
                    if let Some(surface) = floating_surfaces.get_mut(&id) {
                        surface.width = width.max(1);
                        surface.height = height.max(1);
                        let (physical_width, physical_height) = auxiliary_physical_size(
                            surface.width,
                            surface.height,
                            client.scale_120(),
                        );
                        if let Some(renderer) = &mut surface.renderer {
                            renderer
                                .backend_mut()
                                .resize(physical_width, physical_height);
                        } else {
                            let target = client.floating_window_target(id).ok_or_else(|| {
                                "configured floating surface disappeared".to_owned()
                            })?;
                            let backend = pollster::block_on(WgpuBackend::new_surface(
                                target,
                                physical_width,
                                physical_height,
                            ))
                            .map_err(|error| error.to_string())?;
                            surface.renderer = Some(RenderEngine::new(backend));
                        }
                        paint_floating_surface(&runtime, &client, surface)?;
                    }
                }
                LayerEvent::FloatingFrame { id, .. } => {
                    if let Some(surface) = floating_surfaces.get_mut(&id) {
                        paint_floating_surface(&runtime, &client, surface)?;
                    }
                }
                LayerEvent::FloatingClose { id } => {
                    if let Some(surface) = floating_surfaces.remove(&id) {
                        runtime.set_window_surface_visible(surface.id, false);
                    }
                }
                LayerEvent::Key { pressed: false, .. }
                | LayerEvent::Modifiers { .. }
                | LayerEvent::SessionLocked
                | LayerEvent::SessionLockFinished
                | LayerEvent::SessionLockConfigure { .. }
                | LayerEvent::SessionLockSurfaceRemoved { .. }
                | LayerEvent::SessionLockFrame { .. } => {}
                LayerEvent::Screens(screens) => {
                    tx.send(SupervisorMessage::Worker(WorkerMessage::Screens {
                        output: name.clone(),
                        screens,
                    }))
                    .map_err(|_| "output supervisor stopped".to_owned())?;
                }
            }
        }
        apply_clipboard_requests(&mut runtime, &mut client);
        apply_screencopy_requests(&mut runtime, &mut client);
        apply_virtual_keyboard_requests(&mut runtime, &mut client);
        apply_input_method_requests(&mut runtime, &mut client);
        apply_text_input_requests(&mut runtime, &mut client);
        apply_window_surface_actions(&mut runtime, &client, &floating_surfaces);
        if repaint {
            apply_parent_transitions(&mut runtime, &mut renderer, &client)?;
            layout = paint(&runtime, &mut renderer, &client)?;
            for surface in popup_surfaces.values_mut() {
                paint_popup_surface(&runtime, &client, surface)?;
            }
            for surface in floating_surfaces.values_mut() {
                paint_floating_surface(&runtime, &client, surface)?;
            }
        }
    }
}

fn apply_window_surface_actions(
    runtime: &mut Runtime,
    client: &LayerClient,
    floatings: &HashMap<u64, AuxiliarySurface>,
) {
    for action in runtime.take_window_surface_actions() {
        match action {
            WindowSurfaceAction::Move { id } if floatings.contains_key(&id) => {
                client.start_floating_move(id);
            }
            WindowSurfaceAction::Resize { id, edge } if floatings.contains_key(&id) => {
                let edge = match edge.as_str() {
                    "top" => FloatingResizeEdge::Top,
                    "bottom" => FloatingResizeEdge::Bottom,
                    "left" => FloatingResizeEdge::Left,
                    "right" => FloatingResizeEdge::Right,
                    "top_left" => FloatingResizeEdge::TopLeft,
                    "top_right" => FloatingResizeEdge::TopRight,
                    "bottom_left" => FloatingResizeEdge::BottomLeft,
                    "bottom_right" => FloatingResizeEdge::BottomRight,
                    _ => continue,
                };
                client.start_floating_resize(id, edge);
            }
            WindowSurfaceAction::Move { .. } | WindowSurfaceAction::Resize { .. } => {}
        }
    }
}

fn apply_parent_transitions(
    runtime: &mut Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
) -> Result<(), String> {
    let transitions = runtime.take_parent_transitions();
    if transitions.is_empty() {
        return Ok(());
    }
    let root = primary_surface_root(runtime)?;
    let (width, height) = client.logical_size();
    let available = Size {
        width: width as f64,
        height: height as f64,
    };
    for transition in transitions {
        Layout::transition_reparent(
            &mut runtime.scene_mut(),
            renderer.backend_mut(),
            ReparentTransition {
                root,
                node: transition.node,
                new_parent: transition.parent,
                anchors: transition.anchors,
                available,
                behavior: transition.behavior,
            },
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn paint(
    runtime: &Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
) -> Result<Layout, String> {
    let scene = runtime.scene();
    let root = primary_surface_root(runtime)?;
    let (width, height) = client.logical_size();
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
    let (physical_width, physical_height) = client.physical_size();
    if let Some(regions) = runtime.layer_surface_config().input_regions {
        client
            .set_composed_input_region(&regions)
            .map_err(|error| error.to_string())?;
    } else {
        let input = layout
            .input_geometry(&scene)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|geometry| {
                let left = geometry.x.floor() as i32;
                let top = geometry.y.floor() as i32;
                let right = (geometry.x + geometry.width).ceil() as i32;
                let bottom = (geometry.y + geometry.height).ceil() as i32;
                InputRect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                }
            })
            .collect::<Vec<_>>();
        client.set_input_region(Some(&input));
    }
    client.request_frame();
    client
        .surface()
        .damage_buffer(0, 0, physical_width as i32, physical_height as i32);
    let damage = renderer
        .render(&scene, &layout, client.scale_120())
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        client.commit();
    }
    drop(scene);
    runtime.observe_layout(&layout);
    Ok(layout)
}

fn paint_popup_surface(
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    let Some(renderer) = &mut surface.renderer else {
        return Ok(());
    };
    let scene = runtime.scene();
    let layout = Layout::compute(
        &scene,
        surface.root,
        Size {
            width: surface.width as f64,
            height: surface.height as f64,
        },
        renderer.backend_mut(),
    )
    .map_err(|error| error.to_string())?;
    let (width, height) =
        auxiliary_physical_size(surface.width, surface.height, client.scale_120());
    client.request_popup_frame(surface.id);
    let popup = client
        .popup_surface(surface.id)
        .ok_or_else(|| "popup surface disappeared while painting".to_owned())?;
    popup.damage_buffer(0, 0, width as i32, height as i32);
    let damage = renderer
        .render(&scene, &layout, client.scale_120())
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        popup.commit();
    }
    drop(scene);
    runtime.observe_layout(&layout);
    surface.layout = Some(layout);
    Ok(())
}

fn paint_floating_surface(
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    let Some(renderer) = &mut surface.renderer else {
        return Ok(());
    };
    let scene = runtime.scene();
    let layout = Layout::compute(
        &scene,
        surface.root,
        Size {
            width: surface.width as f64,
            height: surface.height as f64,
        },
        renderer.backend_mut(),
    )
    .map_err(|error| error.to_string())?;
    let (width, height) =
        auxiliary_physical_size(surface.width, surface.height, client.scale_120());
    client.request_floating_frame(surface.id);
    let floating = client
        .floating_surface(surface.id)
        .ok_or_else(|| "floating surface disappeared while painting".to_owned())?;
    floating.damage_buffer(0, 0, width as i32, height as i32);
    let damage = renderer
        .render(&scene, &layout, client.scale_120())
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        floating.commit();
    }
    drop(scene);
    runtime.observe_layout(&layout);
    surface.layout = Some(layout);
    Ok(())
}

fn clock_text() -> String {
    jiff::Zoned::now().strftime("%H:%M:%S").to_string()
}

fn until_next_second() -> Duration {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Duration::from_nanos(1_000_000_000 - elapsed.subsec_nanos() as u64)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mold: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_screen_set_tracks_hotplug_identity() {
        let screens = [
            ScreenInfo {
                id: 7,
                name: Some("eDP-1".to_owned()),
                position: Some((0, 0)),
                size: Some((1920, 1080)),
                scale: 1,
                ..ScreenInfo::default()
            },
            ScreenInfo {
                id: 9,
                name: Some("DP-2".to_owned()),
                position: Some((1920, 0)),
                size: Some((2560, 1440)),
                scale: 2,
                ..ScreenInfo::default()
            },
        ];

        let names = named_screens(&screens).unwrap();

        assert_eq!(names.keys().cloned().collect::<Vec<_>>(), ["DP-2", "eDP-1"]);
        assert_eq!(names["DP-2"].id, 9);
    }

    #[test]
    fn primary_root_excludes_registered_window_roots() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "window-roots.lua",
                br#"
                    local ui = require("mold.ui")
                    local window = require("mold.window")
                    local primary = ui.Item {}
                    local popup = ui.Item {}
                    window.popup { root = popup, width = 20, height = 10 }
                "#,
            )
            .unwrap();
        let primary = primary_surface_root(&runtime).unwrap();
        assert_eq!(runtime.scene().roots()[0], primary);
        assert_eq!(auxiliary_physical_size(101, 31, 150), (127, 39));
    }

    #[test]
    fn child_window_visibility_follows_parent_chain() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "window-parents.lua",
                br#"
                    local ui = require("mold.ui")
                    local window = require("mold.window")
                    local parent = window.floating {
                      root = ui.Item {}, visible = false,
                    }
                    local child = window.floating {
                      root = ui.Item {}, visible = true, parent = parent,
                    }
                    window.popup {
                      root = ui.Item {}, visible = true, parent = child,
                    }
                "#,
            )
            .unwrap();
        let surfaces = runtime.window_surface_configs();
        let by_id = surfaces
            .iter()
            .map(|surface| (surface.id, surface))
            .collect::<HashMap<_, _>>();

        assert!(!window_surface_effectively_visible(
            2,
            &by_id,
            &mut HashSet::new()
        ));
        runtime.set_window_surface_visible(0, true);
        let surfaces = runtime.window_surface_configs();
        let by_id = surfaces
            .iter()
            .map(|surface| (surface.id, surface))
            .collect::<HashMap<_, _>>();
        assert!(window_surface_effectively_visible(
            2,
            &by_id,
            &mut HashSet::new()
        ));
    }

    #[test]
    fn command_parser_exposes_ipc_and_legacy_config_path() {
        let args = ["ipc", "call", "launcher.toggle", "one", "two"].map(std::ffi::OsString::from);
        let Command::Client(IpcRequest::Call { target, args }) = parse_command(&args).unwrap()
        else {
            panic!("expected IPC call");
        };
        assert_eq!(target, "launcher.toggle");
        assert_eq!(
            args,
            [
                WireValue::String("one".into()),
                WireValue::String("two".into())
            ]
        );

        let args = [std::ffi::OsString::from("custom.lua")];
        let Command::Run(path, policy) = parse_command(&args).unwrap() else {
            panic!("expected config path");
        };
        assert_eq!(path, PathBuf::from("custom.lua"));
        assert_eq!(policy, LoadPolicy::default());

        let args = ["--no-plugin", "custom.lua"].map(std::ffi::OsString::from);
        let Command::Run(_, policy) = parse_command(&args).unwrap() else {
            panic!("expected config path");
        };
        assert!(!policy.plugins);
        assert!(policy.external_roots);

        let args = ["--clean", "custom.lua"].map(std::ffi::OsString::from);
        let Command::Run(_, policy) = parse_command(&args).unwrap() else {
            panic!("expected config path");
        };
        assert!(!policy.plugins);
        assert!(!policy.external_roots);

        let args = ["lock", "secure.lua"].map(std::ffi::OsString::from);
        let Command::Lock(path) = parse_command(&args).unwrap() else {
            panic!("expected lock config path");
        };
        assert_eq!(path, PathBuf::from("secure.lua"));

        let args = ["log", "--bindings"].map(std::ffi::OsString::from);
        assert!(matches!(
            parse_command(&args).unwrap(),
            Command::Client(IpcRequest::Bindings)
        ));
    }

    #[test]
    fn runtimepath_snapshot_tracks_nested_lua_changes() {
        let root = std::env::temp_dir().join(format!("mold-watch-{}", std::process::id()));
        let module = root.join("lua/plugin/widget.lua");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, b"return 1").unwrap();
        let before = lua_snapshot(std::slice::from_ref(&root));

        fs::write(&module, b"return 200").unwrap();
        let after = lua_snapshot(std::slice::from_ref(&root));

        assert_ne!(before, after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_executes_plugins_before_shell_and_after_last() {
        let root = std::env::temp_dir().join(format!("mold-plugins-{}", std::process::id()));
        fs::create_dir_all(root.join("plugin")).unwrap();
        fs::create_dir_all(root.join("after/plugin/nested")).unwrap();
        fs::write(root.join("plugin/first.lua"), b"plugin_value = 40").unwrap();
        fs::write(
            root.join("after/plugin/nested/last.lua"),
            b"assert(shell_value == 42); after_value = 43",
        )
        .unwrap();
        let shell = root.join("shell.lua");
        let source = b"assert(plugin_value == 40); shell_value = 42; mold.ui.Item {}";
        let mut runtime = Runtime::default();

        execute_config(&mut runtime, &shell, source, LoadPolicy::default()).unwrap();

        assert_eq!(runtime.scene().roots().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn plugin_failures_do_not_stop_later_plugins() {
        let root = std::env::temp_dir().join(format!("mold-plugin-errors-{}", std::process::id()));
        fs::create_dir_all(root.join("plugin")).unwrap();
        fs::write(root.join("plugin/01-broken.lua"), b"error('broken')").unwrap();
        fs::write(root.join("plugin/02-working.lua"), b"plugin_value = 42").unwrap();
        let shell = root.join("shell.lua");
        let mut runtime = Runtime::default();

        execute_config(
            &mut runtime,
            &shell,
            b"assert(plugin_value == 42); mold.ui.Item {}",
            LoadPolicy::default(),
        )
        .unwrap();

        assert_eq!(runtime.scene().roots().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_plugin_policy_skips_discovered_plugins() {
        let root = std::env::temp_dir().join(format!("mold-no-plugin-{}", std::process::id()));
        fs::create_dir_all(root.join("plugin")).unwrap();
        fs::write(root.join("plugin/entry.lua"), b"plugin_loaded = true").unwrap();
        let shell = root.join("shell.lua");
        let mut runtime = Runtime::default();

        execute_config(
            &mut runtime,
            &shell,
            b"assert(plugin_loaded == nil); mold.ui.Item {}",
            LoadPolicy {
                plugins: false,
                external_roots: true,
            },
        )
        .unwrap();

        assert_eq!(runtime.scene().roots().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clean_policy_keeps_only_the_config_root() {
        let config = PathBuf::from("/tmp/mold-clean/shell.lua");

        assert_eq!(
            runtimepath_roots(&config, false),
            [PathBuf::from("/tmp/mold-clean")]
        );
    }

    #[test]
    fn plugin_path_preserves_root_order() {
        let base = std::env::temp_dir().join(format!("mold-plugin-order-{}", std::process::id()));
        let first = base.join("z-first/plugin");
        let second = base.join("a-second/plugin");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("entry.lua"), b"").unwrap();
        fs::write(second.join("entry.lua"), b"").unwrap();

        let scripts = runtime_scripts(&[base.join("z-first"), base.join("a-second")], "plugin");

        assert!(scripts[0].starts_with(base.join("z-first")));
        assert!(scripts[1].starts_with(base.join("a-second")));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn successful_reload_carries_opt_in_state() {
        let screen = Screen {
            name: "test".into(),
            width: None,
            height: None,
            scale: 1,
            ..Screen::default()
        };
        let source = br#"
            local value = mold.reloadable("counter", 0)
            local completed = false
            mold.on_reload_completed(function() completed = true end)
            mold.ipc["counter.set"] = function(next) value:set(next) end
            mold.ipc["counter.get"] = function() return value:get() end
            mold.ipc["reload.completed"] = function() return completed end
            mold.ui.Item {}
        "#;
        let mut runtime = Runtime::for_screen(Limits::default(), screen.clone());
        runtime.execute("shell.lua", source).unwrap();
        runtime
            .call_ipc("counter.set", &[IpcValue::Integer(7)])
            .unwrap();
        let (reply, result) = mpsc::sync_channel(1);

        let update = handle_worker_command(
            &mut runtime,
            &screen,
            LoadPolicy::default(),
            WorkerCommand::Reload {
                path: Arc::new(PathBuf::from("shell.lua")),
                source: Arc::from(&source[..]),
                hard: false,
                reply,
            },
        );

        assert!(result.recv().unwrap().is_ok());
        assert!(update.repaint);
        assert!(!update.recreate_surface);
        assert_eq!(
            runtime.call_ipc("counter.get", &[]).unwrap(),
            [IpcValue::Integer(7)]
        );
        assert_eq!(
            runtime.call_ipc("reload.completed", &[]).unwrap(),
            [IpcValue::Boolean(true)]
        );
    }

    #[test]
    fn hard_reload_discards_opt_in_state() {
        let screen = Screen {
            name: "test".into(),
            width: None,
            height: None,
            scale: 1,
            ..Screen::default()
        };
        let source = br#"
            local value = mold.reloadable("counter", 0)
            mold.ipc["counter.set"] = function(next) value:set(next) end
            mold.ipc["counter.get"] = function() return value:get() end
            mold.ui.Item {}
        "#;
        let mut runtime = Runtime::for_screen(Limits::default(), screen.clone());
        runtime.execute("shell.lua", source).unwrap();
        runtime
            .call_ipc("counter.set", &[IpcValue::Integer(7)])
            .unwrap();
        let (reply, result) = mpsc::sync_channel(1);

        let update = handle_worker_command(
            &mut runtime,
            &screen,
            LoadPolicy::default(),
            WorkerCommand::Reload {
                path: Arc::new(PathBuf::from("shell.lua")),
                source: Arc::from(&source[..]),
                hard: true,
                reply,
            },
        );

        assert!(result.recv().unwrap().is_ok());
        assert!(update.repaint);
        assert!(update.recreate_surface);
        assert_eq!(
            runtime.call_ipc("counter.get", &[]).unwrap(),
            [IpcValue::Integer(0)]
        );
    }

    #[test]
    fn failed_reload_keeps_the_previous_runtime() {
        let screen = Screen {
            name: "test".into(),
            width: None,
            height: None,
            scale: 1,
            ..Screen::default()
        };
        let mut runtime = Runtime::for_screen(Limits::default(), screen.clone());
        runtime
            .execute(
                "shell.lua",
                br#"
                    local failure = ""
                    mold.on_reload_failed(function(error) failure = error end)
                    mold.ipc.value = function() return 7 end
                    mold.ipc["reload.failure"] = function() return failure end
                    mold.ui.Item {}
                "#,
            )
            .unwrap();
        let (reply, result) = mpsc::sync_channel(1);

        let update = handle_worker_command(
            &mut runtime,
            &screen,
            LoadPolicy::default(),
            WorkerCommand::Reload {
                path: Arc::new(PathBuf::from("shell.lua")),
                source: Arc::from(&b"local ="[..]),
                hard: false,
                reply,
            },
        );

        assert!(result.recv().unwrap().is_err());
        assert!(!update.repaint);
        assert!(!update.recreate_surface);
        assert_eq!(
            runtime.call_ipc("value", &[]).unwrap(),
            [IpcValue::Integer(7)]
        );
        let failure = runtime.call_ipc("reload.failure", &[]).unwrap();
        assert!(matches!(&failure[..], [IpcValue::String(error)] if !error.is_empty()));
    }

    #[test]
    fn supervisor_dispatches_registered_ipc_handler() {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (commands, rx) = mpsc::channel();
        let join = thread::spawn(move || {
            let mut runtime = Runtime::default();
            runtime
                .execute(
                    "ipc.lua",
                    b"mold.ipc.echo = function(value) return value, 2 end",
                )
                .unwrap();
            while !worker_stop.load(Ordering::Acquire) {
                if let Ok(command) = rx.recv_timeout(Duration::from_millis(10)) {
                    handle_worker_command(
                        &mut runtime,
                        &Screen {
                            name: "test".into(),
                            width: None,
                            height: None,
                            scale: 1,
                            ..Screen::default()
                        },
                        LoadPolicy::default(),
                        command,
                    );
                }
            }
        });
        let workers = BTreeMap::from([(
            "test".to_owned(),
            Worker {
                stop,
                commands,
                join,
                screen: ScreenInfo {
                    name: Some("test".to_owned()),
                    ..ScreenInfo::default()
                },
            },
        )]);

        let reply = handle_ipc(
            &workers,
            &mut Vec::new(),
            &IpcRequest::Call {
                target: "echo".into(),
                args: vec![WireValue::String("ready".into())],
            },
        );
        assert_eq!(
            reply,
            IpcReply::success(vec![
                WireValue::String("ready".into()),
                WireValue::Integer(2)
            ])
        );

        let (reload, result) = mpsc::sync_channel(1);
        workers["test"]
            .commands
            .send(WorkerCommand::Reload {
                path: Arc::new(PathBuf::from("shell.lua")),
                source: Arc::from(&b"this is not lua"[..]),
                hard: false,
                reply: reload,
            })
            .unwrap();
        assert!(
            result
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_err()
        );
        let reply = handle_ipc(
            &workers,
            &mut Vec::new(),
            &IpcRequest::Call {
                target: "echo".into(),
                args: vec![WireValue::String("last-good".into())],
            },
        );
        assert_eq!(
            reply,
            IpcReply::success(vec![
                WireValue::String("last-good".into()),
                WireValue::Integer(2)
            ])
        );
        stop_workers(workers);
    }
}
