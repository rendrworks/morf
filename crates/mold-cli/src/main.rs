use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mold_layout::{Layout, ReparentTransition, Size};
use mold_lua::{Limits, Runtime, Screen, UiEvent};
use mold_render::{RenderEngine, WgpuBackend};
use mold_wayland::{BarConfig, InputRect, LayerClient, LayerEvent, ScreenInfo};

fn usage() -> &'static str {
    "mold - reactive Wayland shell runtime\n\nusage: mold <shell.lua>\n       mold --help\n       mold --version"
}

fn run() -> Result<(), String> {
    let mut args = env::args_os();
    let _program = args.next();
    let Some(argument) = args.next() else {
        return Err(usage().to_owned());
    };
    if args.next().is_some() {
        return Err("mold accepts exactly one configuration path".to_owned());
    }

    if argument == "-h" || argument == "--help" {
        println!("{}", usage());
        return Ok(());
    }
    if argument == "-V" || argument == "--version" {
        println!("mold {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let path = PathBuf::from(argument);
    let source =
        fs::read(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    supervise(path, source)
}

struct Worker {
    stop: Arc<AtomicBool>,
    join: JoinHandle<()>,
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

fn supervise(path: PathBuf, source: Vec<u8>) -> Result<(), String> {
    let probe = LayerClient::connect(BarConfig::default()).map_err(|error| error.to_string())?;
    let mut desired = named_screens(probe.screens())?;
    drop(probe);
    if desired.is_empty() {
        return Err("compositor advertised no named outputs".to_owned());
    }
    let path = Arc::new(path);
    let source: Arc<[u8]> = source.into();
    let (tx, rx) = mpsc::channel();
    let mut workers = BTreeMap::new();
    reconcile_workers(
        &mut workers,
        &desired,
        Arc::clone(&path),
        Arc::clone(&source),
        &tx,
    );

    loop {
        match rx.recv() {
            Ok(WorkerMessage::Screens { output, screens }) if workers.contains_key(&output) => {
                desired = named_screens(&screens)?;
                reconcile_workers(
                    &mut workers,
                    &desired,
                    Arc::clone(&path),
                    Arc::clone(&source),
                    &tx,
                );
            }
            Ok(WorkerMessage::Screens { .. }) => {}
            Ok(WorkerMessage::Failed { output, error }) => {
                stop_workers(workers);
                return Err(format!("output {output}: {error}"));
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

fn reconcile_workers(
    workers: &mut BTreeMap<String, Worker>,
    desired: &BTreeMap<String, ScreenInfo>,
    path: Arc<PathBuf>,
    source: Arc<[u8]>,
    tx: &mpsc::Sender<WorkerMessage>,
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
        let join = thread::spawn(move || {
            if let Err(error) = run_surface(&path, &source, screen, &tx, &worker_stop)
                && !worker_stop.load(Ordering::Acquire)
            {
                let _ = tx.send(WorkerMessage::Failed { output, error });
            }
        });
        workers.insert(name.clone(), Worker { stop, join });
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
    tx: &mpsc::Sender<WorkerMessage>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let name = screen
        .name
        .clone()
        .ok_or_else(|| format!("output {} has no compositor name", screen.id))?;
    let mut runtime = Runtime::for_screen(
        Limits::default(),
        Screen {
            name: name.clone(),
            width: screen.size.map(|size| size.0),
            height: screen.size.map(|size| size.1),
            scale: screen.scale,
        },
    );
    runtime
        .execute(&path.to_string_lossy(), source)
        .map_err(|error| error.to_string())?;
    if runtime.scene().roots().len() != 1 {
        return Err("configuration must create exactly one root item".to_owned());
    }

    let mut client = LayerClient::connect(BarConfig {
        output: Some(name.clone()),
        ..BarConfig::default()
    })
    .map_err(|error| error.to_string())?;
    tx.send(WorkerMessage::Screens {
        output: name.clone(),
        screens: client.screens().to_vec(),
    })
    .map_err(|_| "output supervisor stopped".to_owned())?;
    'configured: loop {
        client.dispatch().map_err(|error| error.to_string())?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::Configure { .. } => break 'configured,
                LayerEvent::Closed => return Err("layer surface was closed".to_owned()),
                LayerEvent::Scale(_)
                | LayerEvent::Frame { .. }
                | LayerEvent::PointerMotion { .. }
                | LayerEvent::PointerLeave
                | LayerEvent::PointerButton { .. }
                | LayerEvent::Key { .. }
                | LayerEvent::Modifiers { .. }
                | LayerEvent::Screens(_)
                | LayerEvent::PopupConfigure { .. }
                | LayerEvent::PopupFrame { .. }
                | LayerEvent::PopupDone
                | LayerEvent::FloatingConfigure { .. }
                | LayerEvent::FloatingFrame { .. }
                | LayerEvent::FloatingClose => {}
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

    let mut last_frame = None;
    let mut hovered = None;
    let mut pressed = None;
    let mut focused = None;
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        client
            .dispatch_timeout(until_next_second().min(Duration::from_millis(100)))
            .map_err(|error| error.to_string())?;
        let next_clock = clock_text();
        let mut repaint = false;
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
                }
                LayerEvent::PointerLeave => {
                    if let Some(node) = hovered.take() {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerExited);
                    }
                }
                LayerEvent::PointerButton {
                    pressed: true,
                    x,
                    y,
                    ..
                } => {
                    let hit = layout
                        .hit_test(&runtime.scene(), x, y)
                        .map_err(|error| error.to_string())?;
                    pressed = hit;
                    focused = hit;
                    if let Some(node) = hit {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Pressed);
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
                    if let Some(node) = pressed.take() {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
                        if hit == Some(node) {
                            repaint |= runtime.dispatch_ui_event(node, UiEvent::Clicked);
                        }
                    }
                }
                LayerEvent::Key { pressed: true, .. } => {
                    if let Some(node) = focused {
                        repaint |= runtime.dispatch_ui_event(node, UiEvent::KeyPressed);
                    }
                }
                LayerEvent::Key { pressed: false, .. }
                | LayerEvent::Modifiers { .. }
                | LayerEvent::PopupConfigure { .. }
                | LayerEvent::PopupFrame { .. }
                | LayerEvent::PopupDone
                | LayerEvent::FloatingConfigure { .. }
                | LayerEvent::FloatingFrame { .. }
                | LayerEvent::FloatingClose => {}
                LayerEvent::Screens(screens) => {
                    tx.send(WorkerMessage::Screens {
                        output: name.clone(),
                        screens,
                    })
                    .map_err(|_| "output supervisor stopped".to_owned())?;
                }
            }
        }
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
            renderer.backend_mut().text_mut(),
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
        renderer.backend_mut().text_mut(),
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
}
