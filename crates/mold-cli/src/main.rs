use std::collections::{BTreeMap, HashMap};
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
use mold_lua::{IpcValue, Limits, Runtime, Screen, UiEvent};
use mold_render::{RenderEngine, WgpuBackend};
use mold_scene::{Element, NodeHandle};
use mold_wayland::{BarConfig, InputRect, LayerClient, LayerEvent, OutputPowerMode, ScreenInfo};

fn usage() -> &'static str {
    "mold - reactive Wayland shell runtime\n\nusage: mold [shell.lua]\n       mold -c <name>\n       mold lock [lock.lua]\n       mold ipc call <target> [args...]\n       mold ipc verbs\n       mold log [--bindings]\n       mold kill\n       mold --help\n       mold --version"
}

fn run() -> Result<(), String> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    match parse_command(&args)? {
        Command::Help => println!("{}", usage()),
        Command::Version => println!("mold {}", env!("CARGO_PKG_VERSION")),
        Command::Run(path) => {
            let source = fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            supervise(path, source)?;
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
    Run(PathBuf),
    Lock(PathBuf),
    Client(IpcRequest),
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
    match strings.as_slice() {
        ["-h" | "--help"] => Ok(Command::Help),
        ["-V" | "--version"] => Ok(Command::Version),
        ["-c", name] => Ok(Command::Run(named_config_path(name)?)),
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
        [] => Ok(Command::Run(config_root()?.join("shell.lua"))),
        [path] => Ok(Command::Run(PathBuf::from(path))),
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
        reply: mpsc::SyncSender<Result<(), String>>,
    },
}

enum SupervisorMessage {
    Worker(WorkerMessage),
    Ipc(IpcIncoming),
    Reload,
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
    execute_config(&mut runtime, path, source)?;
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
                LayerEvent::Key { pressed: false, .. }
                | LayerEvent::Modifiers { .. }
                | LayerEvent::Configure { .. }
                | LayerEvent::Scale(_)
                | LayerEvent::Frame { .. }
                | LayerEvent::PointerMotion { .. }
                | LayerEvent::PointerLeave
                | LayerEvent::PointerButton { .. }
                | LayerEvent::TouchDown { .. }
                | LayerEvent::TouchMotion { .. }
                | LayerEvent::TouchUp { .. }
                | LayerEvent::TouchCancel
                | LayerEvent::PopupConfigure { .. }
                | LayerEvent::PopupFrame { .. }
                | LayerEvent::PopupDone
                | LayerEvent::FloatingConfigure { .. }
                | LayerEvent::FloatingFrame { .. }
                | LayerEvent::FloatingClose
                | LayerEvent::Closed => {}
            }
        }
        apply_clipboard_requests(&mut runtime, &mut client);
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
    let root = runtime.scene().roots()[0];
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
    Ok(())
}

fn supervise(path: PathBuf, source: Vec<u8>) -> Result<(), String> {
    let probe = LayerClient::connect(BarConfig::default()).map_err(|error| error.to_string())?;
    let mut desired = named_screens(probe.screens())?;
    drop(probe);
    if desired.is_empty() {
        return Err("compositor advertised no named outputs".to_owned());
    }
    let path = Arc::new(path);
    let mut source: Arc<[u8]> = source.into();
    let (tx, rx) = mpsc::channel();
    let reload_roots = runtimepath_roots(&path);
    let mut reload_snapshot = lua_snapshot(&reload_roots);
    let reload_tx = tx.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(100));
            let next = lua_snapshot(&reload_roots);
            if next != reload_snapshot {
                reload_snapshot = next;
                if reload_tx.send(SupervisorMessage::Reload).is_err() {
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
            Ok(SupervisorMessage::Reload) => match fs::read(path.as_ref()) {
                Ok(bytes) => {
                    source = Arc::from(bytes);
                    for (output, worker) in &workers {
                        let (reply, result) = mpsc::sync_channel(1);
                        if worker
                            .commands
                            .send(WorkerCommand::Reload {
                                path: Arc::clone(&path),
                                source: Arc::clone(&source),
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

fn runtimepath_roots(config: &Path) -> Vec<PathBuf> {
    let mut roots = config
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect::<Vec<_>>();
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
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/lua"));
    let mut unique = Vec::new();
    for root in roots {
        if !unique.contains(&root) {
            unique.push(root);
        }
    }
    unique
}

fn execute_config(runtime: &mut Runtime, path: &Path, source: &[u8]) -> Result<(), String> {
    let roots = runtimepath_roots(path);
    for plugin in runtime_scripts(&roots, "plugin") {
        let source = fs::read(&plugin)
            .map_err(|error| format!("could not read {}: {error}", plugin.display()))?;
        runtime
            .execute(&plugin.to_string_lossy(), &source)
            .map_err(|error| error.to_string())?;
    }
    runtime
        .execute(&path.to_string_lossy(), source)
        .map_err(|error| error.to_string())?;
    for after in runtime_scripts(&roots, "after") {
        let source = fs::read(&after)
            .map_err(|error| format!("could not read {}: {error}", after.display()))?;
        runtime
            .execute(&after.to_string_lossy(), &source)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn runtime_scripts(roots: &[PathBuf], directory: &str) -> Vec<PathBuf> {
    let mut scripts = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root.join(directory)) else {
            continue;
        };
        scripts.extend(entries.flatten().filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|value| value.to_str()) == Some("lua")).then_some(path)
        }));
    }
    scripts.sort();
    scripts
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
    tx: &mpsc::Sender<SupervisorMessage>,
) {
    let stale = workers
        .keys()
        .filter(|name| !desired.contains_key(*name))
        .cloned()
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
        let path = Arc::clone(&path);
        let source = Arc::clone(&source);
        let tx = tx.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (commands, command_rx) = mpsc::channel();
        let join = thread::spawn(move || {
            if let Err(error) = run_surface(&path, &source, screen, &tx, &worker_stop, &command_rx)
                && !worker_stop.load(Ordering::Acquire)
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
}

fn handle_worker_command(
    runtime: &mut Runtime,
    screen: &Screen,
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
            reply,
        } => {
            let mut candidate = Runtime::for_screen(Limits::default(), screen.clone());
            candidate.restore_reloadable_state(runtime.reloadable_state());
            let result = execute_config(&mut candidate, &path, &source)
                .and_then(|()| {
                    (candidate.scene().roots().len() == 1)
                        .then_some(())
                        .ok_or_else(|| "configuration must create exactly one root item".to_owned())
                })
                .and_then(|()| {
                    candidate
                        .update_clock(clock_text())
                        .map_err(|error| error.to_string())
                });
            if result.is_ok() {
                *runtime = candidate;
            }
            let repaint = result.is_ok();
            let _ = reply.send(result);
            WorkerUpdate {
                repaint,
                reset_input: repaint,
                refresh_idle: repaint,
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

fn stop_workers(workers: BTreeMap<String, Worker>) {
    for worker in workers.values() {
        worker.stop.store(true, Ordering::Release);
    }
    for (_, worker) in workers {
        let _ = worker.join.join();
    }
}

fn run_surface(
    path: &Path,
    source: &[u8],
    screen: ScreenInfo,
    tx: &mpsc::Sender<SupervisorMessage>,
    stop: &AtomicBool,
    commands: &mpsc::Receiver<WorkerCommand>,
) -> Result<(), String> {
    let name = screen
        .name
        .clone()
        .ok_or_else(|| format!("output {} has no compositor name", screen.id))?;
    let runtime_screen = Screen {
        name: name.clone(),
        width: screen.size.map(|size| size.0),
        height: screen.size.map(|size| size.1),
        scale: screen.scale,
    };
    let mut runtime = Runtime::for_screen(Limits::default(), runtime_screen.clone());
    execute_config(&mut runtime, path, source)?;
    if runtime.scene().roots().len() != 1 {
        return Err("configuration must create exactly one root item".to_owned());
    }

    let mut client = LayerClient::connect(BarConfig {
        output: Some(name.clone()),
        ..BarConfig::default()
    })
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
                LayerEvent::Scale(_)
                | LayerEvent::Idle { .. }
                | LayerEvent::OutputPower { .. }
                | LayerEvent::Clipboard { .. }
                | LayerEvent::Frame { .. }
                | LayerEvent::PointerMotion { .. }
                | LayerEvent::PointerLeave
                | LayerEvent::PointerButton { .. }
                | LayerEvent::TouchDown { .. }
                | LayerEvent::TouchMotion { .. }
                | LayerEvent::TouchUp { .. }
                | LayerEvent::TouchCancel
                | LayerEvent::Key { .. }
                | LayerEvent::Modifiers { .. }
                | LayerEvent::Screens(_)
                | LayerEvent::PopupConfigure { .. }
                | LayerEvent::PopupFrame { .. }
                | LayerEvent::PopupDone
                | LayerEvent::FloatingConfigure { .. }
                | LayerEvent::FloatingFrame { .. }
                | LayerEvent::FloatingClose
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
    apply_output_power_requests(&mut runtime, &mut client);

    let mut last_frame = None;
    let mut hovered = None;
    let mut pressed = None::<(NodeHandle, f64, f64, bool)>;
    let mut focused = runtime.first_key_target();
    let mut touches = HashMap::<i32, (NodeHandle, f64, f64)>::new();
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        client
            .dispatch_timeout(until_next_second().min(Duration::from_millis(100)))
            .map_err(|error| error.to_string())?;
        let next_clock = clock_text();
        let mut repaint = runtime.poll_services();
        while let Ok(command) = commands.try_recv() {
            let update = handle_worker_command(&mut runtime, &runtime_screen, command);
            repaint |= update.repaint;
            if update.reset_input {
                hovered = None;
                pressed = None;
                focused = runtime.first_key_target();
                touches.clear();
            }
            if update.refresh_idle {
                client.set_idle_timeouts(&runtime.idle_timeouts());
            }
        }
        apply_output_power_requests(&mut runtime, &mut client);
        apply_clipboard_requests(&mut runtime, &mut client);
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
                LayerEvent::PointerMotion { x, y } => {
                    let hit = layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    if hit != hovered {
                        if let Some(node) = hovered {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerExited);
                        }
                        if let Some(node) = hit {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerEntered);
                        }
                        hovered = hit;
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
                    if let Some((node, start_x, start_y, dragging)) = &mut pressed {
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
                LayerEvent::PointerLeave => {
                    if let Some(node) = hovered.take() {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerExited);
                    }
                }
                LayerEvent::PointerButton {
                    button,
                    pressed: true,
                    x,
                    y,
                } => {
                    let hit = layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    let hit = hit.filter(|node| runtime.accepts_pointer_button(*node, button));
                    pressed = hit.map(|node| (node, x, y, false));
                    focused = hit.and_then(|node| runtime.key_target_for_node(node));
                    if let Some(node) = hit {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Pressed);
                    }
                }
                LayerEvent::TouchDown { id, x, y } => {
                    let hit = layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    if let Some(node) = hit {
                        touches.insert(id, (node, x, y));
                        focused = runtime.key_target_for_node(node);
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Pressed);
                        repaint |=
                            runtime.dispatch_touch_event(node, UiEvent::TouchPressed, id, x, y);
                    }
                }
                LayerEvent::TouchMotion { id, x, y } => {
                    if let Some((node, last_x, last_y)) = touches.get_mut(&id) {
                        *last_x = x;
                        *last_y = y;
                        repaint |=
                            runtime.dispatch_touch_event(*node, UiEvent::TouchMoved, id, x, y);
                    }
                }
                LayerEvent::TouchUp { id, x, y } => {
                    if let Some((node, _, _)) = touches.remove(&id) {
                        repaint |=
                            runtime.dispatch_touch_event(node, UiEvent::TouchReleased, id, x, y);
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
                        let hit = layout
                            .hit_test(&runtime.scene(), x, y)
                            .map_err(|error| error.to_string())?;
                        if hit == Some(node) {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::Clicked);
                        }
                    }
                }
                LayerEvent::TouchCancel => {
                    for (id, (node, x, y)) in touches.drain() {
                        repaint |=
                            runtime.dispatch_touch_event(node, UiEvent::TouchCanceled, id, x, y);
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
                    }
                }
                LayerEvent::PointerButton {
                    pressed: false,
                    x,
                    y,
                    ..
                } => {
                    let hit = layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    if let Some((node, start_x, start_y, dragging)) = pressed.take() {
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
                        } else if hit == Some(node) {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::Clicked);
                        }
                    }
                }
                LayerEvent::Key {
                    pressed: true,
                    keysym,
                    text,
                    ..
                } => {
                    if keysym == 0xff09 {
                        focused = runtime.next_key_target(focused);
                        repaint = true;
                    } else if let Some(node) = focused {
                        repaint |= runtime.dispatch_key_event(node, keysym, text.as_deref());
                    }
                }
                LayerEvent::Key { pressed: false, .. }
                | LayerEvent::Modifiers { .. }
                | LayerEvent::PopupConfigure { .. }
                | LayerEvent::PopupFrame { .. }
                | LayerEvent::PopupDone
                | LayerEvent::FloatingConfigure { .. }
                | LayerEvent::FloatingFrame { .. }
                | LayerEvent::FloatingClose
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
        if repaint {
            apply_parent_transitions(&mut runtime, &mut renderer, &client)?;
            layout = paint(&runtime, &mut renderer, &client)?;
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
    let root = runtime.scene().roots()[0];
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
    let root = scene.roots()[0];
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
    Ok(layout)
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
            },
            ScreenInfo {
                id: 9,
                name: Some("DP-2".to_owned()),
                position: Some((1920, 0)),
                size: Some((2560, 1440)),
                scale: 2,
            },
        ];

        let names = named_screens(&screens).unwrap();

        assert_eq!(names.keys().cloned().collect::<Vec<_>>(), ["DP-2", "eDP-1"]);
        assert_eq!(names["DP-2"].id, 9);
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
        let Command::Run(path) = parse_command(&args).unwrap() else {
            panic!("expected config path");
        };
        assert_eq!(path, PathBuf::from("custom.lua"));

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
        fs::create_dir_all(root.join("after")).unwrap();
        fs::write(root.join("plugin/first.lua"), b"plugin_value = 40").unwrap();
        fs::write(
            root.join("after/last.lua"),
            b"assert(shell_value == 42); after_value = 43",
        )
        .unwrap();
        let shell = root.join("shell.lua");
        let source = b"assert(plugin_value == 40); shell_value = 42; mold.ui.Item {}";
        let mut runtime = Runtime::default();

        execute_config(&mut runtime, &shell, source).unwrap();

        assert_eq!(runtime.scene().roots().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_reload_carries_opt_in_state() {
        let screen = Screen {
            name: "test".into(),
            width: None,
            height: None,
            scale: 1,
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
            WorkerCommand::Reload {
                path: Arc::new(PathBuf::from("shell.lua")),
                source: Arc::from(&source[..]),
                reply,
            },
        );

        assert!(result.recv().unwrap().is_ok());
        assert!(update.repaint);
        assert_eq!(
            runtime.call_ipc("counter.get", &[]).unwrap(),
            [IpcValue::Integer(7)]
        );
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
                        },
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
