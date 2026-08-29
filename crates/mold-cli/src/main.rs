use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mold_layout::{Layout, Size};
use mold_lua::Runtime;
use mold_render::{RenderEngine, WgpuBackend};
use mold_wayland::{BarConfig, LayerClient, LayerEvent};

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
    let mut runtime = Runtime::default();
    runtime
        .execute(&path.to_string_lossy(), &source)
        .map_err(|error| error.to_string())?;
    if runtime.scene().roots().len() != 1 {
        return Err("configuration must create exactly one root item".to_owned());
    }

    let mut client =
        LayerClient::connect(BarConfig::default()).map_err(|error| error.to_string())?;
    'configured: loop {
        client.dispatch().map_err(|error| error.to_string())?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::Configure { .. } => break 'configured,
                LayerEvent::Closed => return Err("layer surface was closed".to_owned()),
                LayerEvent::Scale(_) | LayerEvent::Frame { .. } => {}
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
    paint(&runtime, &mut renderer, &client)?;

    let mut last_frame = None;
    loop {
        client
            .dispatch_timeout(until_next_second())
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
                LayerEvent::Closed => return Ok(()),
            }
        }
        if repaint {
            paint(&runtime, &mut renderer, &client)?;
        }
    }
}

fn paint(
    runtime: &Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
) -> Result<(), String> {
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
