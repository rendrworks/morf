//! Times the work a frame does on the CPU, without a compositor.
//!
//! Layout, the draw list and the input region are the three things every paint
//! performs before anything reaches the GPU, and all three are pure functions
//! of the scene — so they can be measured exactly, repeatably, and without a
//! display. Run it against any configuration:
//!
//! ```sh
//! cargo run --release -p morf-cli --example frame_bench -- examples/quickshell/init.lua
//! ```
//!
//! Numbers are the fastest of several batches. Background load only ever adds
//! time, so the minimum is the closest estimate of the work itself; an average
//! would mostly measure whatever else the machine was doing.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use morf_layout::{Layout, Size, TextMeasurer, TextOptions};
use morf_lua::{Limits, Runtime, Screen};
use morf_render::{DrawList, RenderEngine, ShaderRegistration, WgpuBackend};
use morf_scene::{Element, NodeHandle};

/// Text measured by a rule rather than a font stack.
///
/// The point is to time layout, not shaping — and shaping is already cached per
/// node behind its own input key, so including it would measure a cache hit.
struct RuledText;

impl TextMeasurer for RuledText {
    fn measure(
        &mut self,
        _node: NodeHandle,
        text: &str,
        _family: &str,
        size: f64,
        _options: TextOptions,
    ) -> Size {
        Size {
            width: text.chars().count() as f64 * size * 0.6,
            height: size * 1.2,
        }
    }

    fn measure_image(
        &mut self,
        _node: NodeHandle,
        _element: Element,
        _source: &str,
        _theme: Option<&str>,
    ) -> Option<Size> {
        None
    }
}

/// The fastest of `batches` runs of `body`, per iteration.
fn best(batches: u32, runs: u32, mut body: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..batches {
        let start = Instant::now();
        for _ in 0..runs {
            body();
        }
        best = best.min(start.elapsed() / runs);
    }
    best
}

fn main() {
    let Some(config) = std::env::args().nth(1) else {
        eprintln!("usage: frame_bench <config.lua> [width] [height]");
        std::process::exit(2);
    };
    let width: f64 = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(3456.0);
    let height: f64 = std::env::args()
        .nth(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(2160.0);

    // A screen, not the default screenless runtime. The documented way to
    // write a configuration is `morf.variants(morf.screens, …)`, which builds
    // nothing at all when there are no screens — so a benchmark that skipped
    // this could not open the configurations most worth benchmarking.
    let mut runtime = Runtime::for_screen(
        Limits::default(),
        Screen {
            name: "frame-bench".into(),
            width: Some(width as i32),
            height: Some(height as i32),
            scale: 1,
            ..Screen::default()
        },
    );
    if let Some(parent) = PathBuf::from(&config).parent() {
        runtime.set_module_roots(vec![parent.to_path_buf()]);
    }
    let source = std::fs::read(&config).expect("configuration is readable");
    if let Err(error) = runtime.execute(&config, &source) {
        eprintln!("{config}: {error}");
        std::process::exit(1);
    }
    // Let the services settle so the scene is the one a running shell has.
    for _ in 0..30 {
        runtime.poll_services();
        runtime
            .tick_animations(Duration::from_millis(16))
            .expect("animations tick");
    }

    // `trace` follows the moving parts instead of timing them, so a
    // configuration whose motion misbehaves can be reproduced without a
    // compositor in the way.
    if std::env::args().nth(2).as_deref() == Some("trace") {
        let mut moving = Vec::new();
        let mut stack = vec![runtime.scene().roots()[0]];
        while let Some(node) = stack.pop() {
            let scene = runtime.scene();
            if format!("{:?}", scene.element(node).expect("live")) == "SdfShape" {
                moving.push(node);
            }
            stack.extend(scene.children(node).expect("live").iter().copied());
        }
        println!("tracing {} shapes", moving.len());
        // In real time, deliberately. Timers fire off the wall clock, so a
        // configuration that applies forces from one — anything with parts
        // that pull on each other — sees those forces only if the trace takes
        // as long to run as the motion it is tracing. Racing through the
        // frames traces the motion with every force switched off.
        // Long enough to show a slow drift, which is the failure a short trace
        // cannot tell apart from an orbit.
        let frames: u64 = std::env::args()
            .nth(3)
            .and_then(|arg| arg.parse().ok())
            .map_or(600, |seconds: u64| seconds * 1000 / 16);
        let started = Instant::now();
        for frame in 0..frames {
            let due = Duration::from_millis(frame * 16);
            if let Some(rest) = due.checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
            runtime.poll_services();
            let advanced = runtime
                .tick_animations(Duration::from_millis(16))
                .expect("tick");
            if frame % (frames / 20).max(1) == 0 || frame + 1 == frames {
                let scene = runtime.scene();
                let placed: Vec<(f64, f64, f64)> = moving
                    .iter()
                    .map(|node| {
                        let size = scene.number(*node, "width").unwrap_or(0.0);
                        (
                            scene.number(*node, "x").unwrap_or(f64::NAN) + size / 2.0,
                            scene.number(*node, "y").unwrap_or(f64::NAN) + size / 2.0,
                            size / 2.0,
                        )
                    })
                    .collect();
                // Three numbers say more about a swarm than its coordinates
                // do: where it sits, how far it reaches, and whether its parts
                // are still distinct or have piled into one lump.
                let count = placed.len() as f64;
                let mid_x = placed.iter().map(|p| p.0).sum::<f64>() / count;
                let mid_y = placed.iter().map(|p| p.1).sum::<f64>() / count;
                let reach = placed
                    .iter()
                    .map(|p| (p.0 - mid_x).hypot(p.1 - mid_y))
                    .fold(0.0, f64::max);
                let mut closest = f64::INFINITY;
                for (index, a) in placed.iter().enumerate() {
                    for b in &placed[index + 1..] {
                        closest = closest.min((a.0 - b.0).hypot(a.1 - b.1) - a.2 - b.2);
                    }
                }
                // How far the worst blob has pushed past the edge of the
                // surface, which is the one failure that shows on screen as a
                // shape sliced flat rather than as motion that looks wrong.
                let root = scene.roots()[0];
                let wide = scene.number(root, "width").unwrap_or(0.0);
                let tall = scene.number(root, "height").unwrap_or(0.0);
                let escaped = placed
                    .iter()
                    .map(|p| {
                        (p.2 - p.0)
                            .max(p.0 + p.2 - wide)
                            .max(p.2 - p.1)
                            .max(p.1 + p.2 - tall)
                    })
                    .fold(f64::NEG_INFINITY, f64::max);
                let spread = format!(
                    "at ({mid_x:.0},{mid_y:.0}) reach {reach:.0} closest {closest:+.0} escaped {escaped:+.0}"
                );
                println!(
                    "  t={:>5}ms active={} {}",
                    started.elapsed().as_millis(),
                    advanced.active,
                    spread
                );
            }
        }
        return;
    }

    // What a shell costs when nothing is animating: the run loop wakes ten
    // times a second and asks the services what happened, whether or not
    // anything is painted afterwards.
    let idle = best(8, 60, || {
        std::hint::black_box(runtime.poll_services());
    });
    let tick = best(8, 60, || {
        std::hint::black_box(
            runtime
                .tick_animations(Duration::from_millis(16))
                .expect("animations tick"),
        );
    });

    let scene = runtime.scene();
    let root = scene.roots()[0];
    let mut nodes = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        nodes += 1;
        stack.extend(scene.children(node).expect("live node").iter().copied());
    }
    let size = Size { width, height };
    let computed = Layout::compute(&scene, root, size, &mut RuledText).expect("layout");

    // `gpu` renders one frame on a real adapter instead of timing anything.
    //
    // Everything above this line runs on the CPU, and a shader that compiles to
    // WGSL the driver then refuses looks exactly the same from up here as one
    // that works: the configuration loads, the scene is built, the numbers come
    // out fine. The only way to find out is to build the pipelines and draw,
    // which is what this does — headless, so it can run over every example
    // without a compositor.
    if std::env::args().nth(2).as_deref() == Some("gpu") {
        let backend = pollster::block_on(WgpuBackend::new(width as u32, height as u32))
            .expect("a GPU adapter");
        let mut engine = RenderEngine::new(backend);
        let mut shaders = 0usize;
        for shader in runtime.shaders() {
            engine
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
                .unwrap_or_else(|error| panic!("{config}: shader pipeline: {error}"));
            shaders += 1;
        }
        // Twice: the first frame damages everything, the second takes the
        // incremental path, and an effect layer's target is only reused on the
        // second.
        for _ in 0..2 {
            engine
                .render(&runtime.scene(), &computed, 120, |_| {})
                .unwrap_or_else(|error| panic!("{config}: render: {error}"));
        }
        println!("{config}");
        println!("  {shaders} shader(s) built and one frame drawn on the GPU");
        // A fourth argument names a PNG to write the frame to. Every gate in
        // this repository can pass while a shader is visibly wrong, and the
        // only way to find that out is to look at what it drew.
        if let Some(path) = std::env::args().nth(3) {
            let pixels = engine.backend_mut().read_pixels();
            image::RgbaImage::from_raw(width as u32, height as u32, pixels)
                .expect("the readback is the size of the target")
                .save(&path)
                .expect("the image is written");
            println!("  written to {path}");
        }
        return;
    }

    // Layout reads about a dozen properties per node, so this is the floor the
    // rest of the pass is built on.
    let mut all = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        all.push(node);
        stack.extend(scene.children(node).expect("live node").iter().copied());
    }
    const PROBED: [&str; 12] = [
        "x",
        "y",
        "width",
        "height",
        "visible",
        "opacity",
        "rotation",
        "scale",
        "implicit_width",
        "implicit_height",
        "z",
        "clip",
    ];
    let reads = best(12, 200, || {
        for &node in &all {
            for name in PROBED {
                std::hint::black_box(scene.number(node, name).ok());
            }
        }
    });

    let layout = best(12, 200, || {
        std::hint::black_box(Layout::compute(&scene, root, size, &mut RuledText).expect("layout"));
    });
    let reuse = best(12, 200, || {
        std::hint::black_box(computed.clone());
    });
    let mut reused = DrawList::default();
    let draw = best(12, 200, || {
        reused.rebuild(&scene, &computed).expect("draw list");
        std::hint::black_box(&reused);
    });
    let draw_fresh = best(12, 200, || {
        std::hint::black_box(DrawList::from_scene(&scene, &computed).expect("draw list"));
    });
    let region = best(12, 200, || {
        std::hint::black_box(computed.input_geometry(&scene).expect("input geometry"));
    });

    // Shaders are counted because the way they fail is silent. A material
    // shader that never reached a command, or an effect whose layer holds
    // nothing to sample, renders as a configuration that simply ignored it —
    // and the only place that is visible without a GPU is here.
    let shaded = DrawList::from_scene(&scene, &computed).expect("draw list");
    let material = shaded
        .commands
        .iter()
        .filter(|command| {
            matches!(
                command,
                morf_render::DrawCommand::Field {
                    shader: Some(_),
                    ..
                } | morf_render::DrawCommand::Quad {
                    shader: Some(_),
                    ..
                }
            )
        })
        .count();
    // What an effect layer holds *other than the node carrying it*. A rectangle
    // laid over its siblings still draws its own quad, so counting commands
    // would say 1 and mean nothing; the question is whether any content came
    // with it.
    let effects: Vec<usize> = shaded
        .layers
        .iter()
        .filter(|layer| layer.shader.is_some())
        .map(|layer| {
            shaded.commands[layer.commands.clone()]
                .iter()
                .filter(|command| command.node() != layer.node)
                .count()
        })
        .collect();

    let frame = layout + draw + region;
    println!("{config}");
    println!("  scene nodes        {nodes}");
    if material > 0 || !effects.is_empty() {
        println!("  shaded commands    {material}");
        for covered in &effects {
            println!(
                "  effect layer       {covered} command(s) to sample{}",
                if *covered == 0 {
                    "  <-- wraps nothing, so it will sample an empty target"
                } else {
                    ""
                },
            );
        }
    }
    println!(
        "  DrawCommand size   {} bytes  ({} KiB for this scene)",
        std::mem::size_of::<morf_render::DrawCommand>(),
        std::mem::size_of::<morf_render::DrawCommand>() * nodes / 1024
    );
    println!("  poll_services      {idle:?}");
    println!(
        "  property read      {:.1}ns  ({} reads)",
        reads.as_secs_f64() * 1e9 / (all.len() * PROBED.len()) as f64,
        all.len() * PROBED.len()
    );
    println!("  tick_animations    {tick:?}");
    println!("  Layout::compute    {layout:?}");
    println!("  Layout reuse       {reuse:?}");
    println!("  DrawList (reused)  {draw:?}");
    println!("  DrawList (fresh)   {draw_fresh:?}");
    println!("  input_geometry     {region:?}");
    println!("  frame CPU          {frame:?}");
    println!(
        "  at 60fps           {:.1}% of one core per output",
        frame.as_secs_f64() * 60.0 * 100.0
    );
}
