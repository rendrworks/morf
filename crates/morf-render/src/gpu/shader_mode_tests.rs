//! Surface and effect shaders, and the language surface, against an adapter.
//!
//! Split from `shader_tests` when the two together crossed the line gate. The
//! ones here need something the material tests do not: their own coverage,
//! something rendered underneath them, or a builtin whose emission a driver has
//! never seen.

use crate::gpu::backend_types::ShaderRegistration;
use crate::gpu::field_tests::{alpha_at, field_command, field_layer, read_frame};
use crate::gpu::shader_tests::{SIZE, channel, shaded};
use crate::*;
use morf_layout::Geometry;
use morf_scene::Color;
use morf_shader::{ShaderKind, ShaderSpec};

/// Renders one surface-mode shader, where the shader decides its own coverage.
pub(super) fn surface(body: &str) -> Vec<u8> {
    let spec = ShaderSpec {
        kind: ShaderKind::Surface,
        inputs: ShaderSpec::default_inputs(ShaderKind::Surface),
        params: Vec::new(),
        entry: "fragment".to_owned(),
        textures: Vec::new(),
        data: Vec::new(),
        vertex: false,
    };
    let compiled = morf_shader::compile(body, &spec)
        .unwrap_or_else(|errors| panic!("{}", morf_shader::report("test", &errors)));
    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    backend
        .register_shader(ShaderRegistration {
            program: compiled.hash,
            wgsl: Some(&compiled.wgsl),
            vertex: None,
            offsets: &[],
            uniform_size: compiled.uniform_size,
            owns_coverage: true,
            effect: false,
            textures: &[],
            data: &[],
        })
        .expect("the generated WGSL compiles");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut command = field_command(node, vec![field_layer(8.0, 8.0, 48.0, Shape::Box)]);
    if let DrawCommand::Field { shader, .. } = &mut command {
        *shader = Some(ShaderBinding {
            program: compiled.hash,
            params: Vec::new(),
            data: Vec::new(),
            samples_behind: false,
            owns_coverage: true,
        });
    }
    let list = DrawList {
        commands: vec![command],
        layers: Vec::new(),
    };
    read_frame(&mut backend, &list, SIZE)
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_surface_shader_decides_its_own_coverage() {
    // The difference between the two modes, in one test. The node's own shape
    // is a plain box; the shader carves a disc out of it by returning alpha,
    // and the corners the box would have filled are clear.
    let pixels = surface(
        "function fragment(uv)
           local d = length(uv - vec2(0.5, 0.5))
           local inside = 1.0 - step(0.3, d)
           return vec4(1.0, 0.2, 0.2, inside)
         end",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 32),
        255,
        "the middle of the disc is painted",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 12, 12),
        0,
        "and the box corner the shader did not claim is clear",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_material_shader_cannot_paint_outside_the_shape_but_a_surface_one_can() {
    // The same body under both modes. A material shader is multiplied by the
    // field's coverage, so its alpha of one is still nothing outside the box; a
    // surface shader's is not, so it fills its whole rectangle.
    let body = "function fragment(uv)
                  return vec4(0.2, 0.9, 0.4, 1.0)
                end";
    let material = shaded(body);
    let surfaced = surface(body);
    // Well outside the eight-to-fifty-six box, inside the node's rectangle.
    assert_eq!(alpha_at(&material, SIZE, 3, 3), 0, "the field clipped it");
    assert_eq!(
        alpha_at(&surfaced, SIZE, 3, 3),
        255,
        "and the surface shader was not clipped",
    );
}

/// Renders one square of `colour` underneath an effect shader and hands back
/// the composited pixels.
///
/// Effect mode needs something rendered into a layer before there is anything
/// to sample, which is most of the setup here and none of what any individual
/// test below is about.
fn through_effect(body: &str, params: &[(&str, f32)], colour: Color) -> Vec<u8> {
    let spec = ShaderSpec {
        kind: ShaderKind::Effect,
        inputs: ShaderSpec::default_inputs(ShaderKind::Effect),
        params: params
            .iter()
            .map(|(name, _)| morf_shader::Binding {
                name: (*name).to_owned(),
                ty: morf_shader::Type::F32,
            })
            .collect(),
        entry: "fragment".to_owned(),
        textures: Vec::new(),
        data: Vec::new(),
        vertex: false,
    };
    let compiled = morf_shader::compile(body, &spec)
        .unwrap_or_else(|errors| panic!("{}", morf_shader::report("effect", &errors)));
    assert!(compiled.samples_behind, "the compiler saw the sample");

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    let offsets: Vec<u32> = compiled.params.iter().map(|slot| slot.offset).collect();
    assert_eq!(offsets.len(), params.len(), "every parameter got a slot");
    backend
        .register_shader(ShaderRegistration {
            program: compiled.hash,
            wgsl: Some(&compiled.wgsl),
            vertex: None,
            offsets: &offsets,
            uniform_size: compiled.uniform_size,
            owns_coverage: false,
            effect: true,
            textures: &[],
            data: &[],
        })
        .expect("the generated WGSL compiles");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut layer = field_layer(4.0, 4.0, 56.0, Shape::Box);
    layer.color = colour;
    let mut command = field_command(node, vec![layer]);
    if let DrawCommand::Field { fill_color, .. } = &mut command {
        *fill_color = colour;
    }
    let list = DrawList {
        commands: vec![command],
        layers: vec![Layer {
            node,
            commands: 0..1,
            parent: None,
            bounds: Geometry {
                x: 0.0,
                y: 0.0,
                width: SIZE as f64,
                height: SIZE as f64,
            },
            opacity: 1.0,
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_offset: [0.0, 0.0],
            mask: None,
            shader: Some(ShaderBinding {
                program: compiled.hash,
                params: params.iter().map(|(_, value)| *value).collect(),
                data: Vec::new(),
                samples_behind: true,
                owns_coverage: false,
            }),
        }],
    };
    read_frame(&mut backend, &list, SIZE)
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn an_effect_shader_reads_what_is_underneath_it() {
    // The mode that needs a layer: until the subtree has been rendered into its
    // own target there is nothing to sample. A red square is drawn, and the
    // effect swaps its channels — so what comes back is blue, which it could
    // only know by reading what was already there.
    let pixels = through_effect(
        "function fragment(uv)
           local under = texture(uv)
           return vec4(under.b, under.g, under.r, under.a)
         end",
        &[],
        Color::rgba8(255, 0, 0, 255),
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 32),
        255,
        "the layer was composited",
    );
    assert_eq!(channel(&pixels, 32, 32, 2), 255, "red arrived as blue");
    assert_eq!(channel(&pixels, 32, 32, 0), 0, "and nothing stayed red");
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn an_effect_shaders_parameters_reach_it() {
    // Effect shaders are registered in their own table, because they splice
    // into a different pipeline, and their parameters live on the layer rather
    // than on any command. Miss either of those and the shader still runs and
    // still compiles — it just reads zero for everything the configuration
    // declared, which is indistinguishable from an effect that does nothing.
    let pixels = through_effect(
        "function fragment(uv, time, resolution, level)
           return vec4(level, 0.0, 0.0, texture(uv).a)
         end",
        &[("level", 1.0)],
        Color::rgba8(255, 255, 255, 255),
    );
    assert_eq!(alpha_at(&pixels, SIZE, 32, 32), 255, "the layer composited");
    assert_eq!(
        channel(&pixels, 32, 32, 0),
        255,
        "and the declared parameter arrived rather than defaulting to zero",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_ported_shadertoy_shader_compiles_and_paints() {
    // Compiling is not the same as being WGSL a driver accepts. This is the
    // Plasma port from `morf-shader`'s port suite, taken all the way to an
    // adapter: vector division, a `tanh` tonemap, swizzled feedback through a
    // loop, and a shape that came from somebody else's shader rather than from
    // what happened to be convenient to emit.
    let pixels = shaded(
        "function fragment(uv, time, resolution)
           local I = uv * resolution
           local r = resolution
           local p = (I + I - r) / r.y
           local O = vec3(0.0, 0.0, 0.0)
           local f = p * (4.0 - 4.0 * abs(0.7 - dot(p, p)))
           local i = 0.0
           while i < 8.0 do
             i = i + 1.0
             local s = sin(f) + vec2(1.0, 1.0)
             O = O + vec3(s.x, s.y, s.y) * abs(f.x - f.y)
             f = f + cos(f.yx * i + vec2(i, i) + vec2(time, time)) / i + vec2(0.7, 0.7)
           end
           O = tanh(7.0 * exp(0.0 - p.y * vec3(-1.0, 1.0, 2.0)) / O)
           return vec4(O, 1.0)
         end",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 32),
        255,
        "the ported shader painted the node",
    );
    // Not a flat fill: a plasma that came out one colour everywhere would pass
    // an alpha check and mean nothing.
    let samples: Vec<[u8; 3]> = [(14, 20), (24, 30), (32, 32), (40, 42), (50, 46)]
        .into_iter()
        .map(|(x, y)| {
            [
                channel(&pixels, x, y, 0),
                channel(&pixels, x, y, 1),
                channel(&pixels, x, y, 2),
            ]
        })
        .collect();
    let distinct = samples
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert!(
        distinct > 1,
        "and it varies across the surface: {samples:?}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_helper_function_survives_to_the_gpu() {
    // iq's palette, the most-copied helper in shader writing. Emitted as a real
    // WGSL function ahead of the entry point, with its parameter type taken
    // from the call because Lua had nowhere to declare one.
    let pixels = shaded(
        "function palette(t)
           local a = vec3(0.5, 0.5, 0.5)
           local d = vec3(0.263, 0.416, 0.557)
           return a + a * cos(6.28318 * (t + d))
         end

         function fragment(uv, time, resolution)
           return vec4(palette(uv.x), 1.0)
         end",
    );
    assert_eq!(alpha_at(&pixels, SIZE, 32, 32), 255, "the node is painted");
    // A palette sweeps: two points across it cannot be the same colour.
    let left = channel(&pixels, 14, 32, 0);
    let right = channel(&pixels, 50, 32, 0);
    assert_ne!(left, right, "and the helper's gradient came through");
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_crt_effect_compiles_and_reworks_what_it_samples() {
    // The shader from `examples/crt-terminal.lua`, which is the most demanding
    // thing the language has been asked to emit in anger: six parameters, a
    // helper taking and returning a vector, five texture taps at computed
    // offsets, integer modulo on a coordinate derived from the resolution, and
    // an if/elseif chain assigning a `vec3`. Compiling is not the same as being
    // WGSL a driver accepts, which is the only reason this test exists.
    let pixels = through_effect(
        "function warp(uv, curve)
           local p = uv * 2.0 - vec2(1.0, 1.0)
           local pull = vec2(p.y * p.y, p.x * p.x) * curve
           return (p + p * pull) * 0.5 + vec2(0.5, 0.5)
         end

         function fragment(uv, time, resolution, curve, glow, lines, mask, roll, scan)
           local bent = warp(uv, curve)
           local on_glass = step(0.0, bent.x) * step(bent.x, 1.0)
             * step(0.0, bent.y) * step(bent.y, 1.0)
           local lit = texture(bent).xyz
           local step_x = 2.0 / resolution.x
           local near = max(
             texture(bent + vec2(step_x, 0.0)).xyz,
             texture(bent - vec2(step_x, 0.0)).xyz
           )
           local far = max(
             texture(bent + vec2(step_x * 2.5, 0.0)).xyz,
             texture(bent - vec2(step_x * 2.5, 0.0)).xyz
           )
           lit = lit + (near * 0.6 + far * 0.3) * glow
           local beam = sin(bent.y * lines * 3.14159265) * 0.5 + 0.5
           lit = lit * (1.0 - scan * beam)
           local stripe = i32(floor(uv.x * resolution.x)) % 3
           local tint = vec3(1.0, 1.0, 1.0)
           if stripe == 0 then
             tint = vec3(1.0, 1.0 - mask, 1.0 - mask)
           elseif stripe == 1 then
             tint = vec3(1.0 - mask, 1.0, 1.0 - mask)
           else
             tint = vec3(1.0 - mask, 1.0 - mask, 1.0)
           end
           lit = lit * tint
           local band = fract(bent.y - time * roll)
           lit = lit + lit * smoothstep(0.94, 1.0, band) * 0.35
           local off = bent - vec2(0.5, 0.5)
           lit = lit * clamp(1.0 - dot(off, off) * 1.15, 0.0, 1.0)
           local a = texture(bent).w
           return vec4(lit * on_glass, a * on_glass)
         end",
        &[
            ("curve", 0.10),
            ("glow", 0.55),
            ("lines", 16.0),
            ("mask", 0.30),
            ("roll", 0.35),
            ("scan", 0.9),
        ],
        Color::rgba8(255, 255, 255, 255),
    );
    assert_eq!(alpha_at(&pixels, SIZE, 32, 32), 255, "the tube composited");

    // The shadow mask alone guarantees three neighbouring columns cannot agree:
    // each is allowed to be bright in a different channel.
    let columns: Vec<[u8; 3]> = (30..33)
        .map(|x| {
            [
                channel(&pixels, x, 32, 0),
                channel(&pixels, x, 32, 1),
                channel(&pixels, x, 32, 2),
            ]
        })
        .collect();
    assert!(
        columns
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1,
        "the shadow mask split the columns: {columns:?}",
    );

    // And the scanlines mean rows a few pixels apart cannot agree either.
    let rows: Vec<u8> = (28..36).map(|y| channel(&pixels, 32, y, 1)).collect();
    assert!(
        rows.iter().collect::<std::collections::HashSet<_>>().len() > 1,
        "the beam drew lines: {rows:?}",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_chromatic_effect_pulls_the_channels_apart() {
    // The shader from `examples/sdf-blobs-chroma.lua`: a 32-bit hash for the
    // tear, three texture taps at different offsets, and an alpha taken from
    // whichever channel found something. A white square in, and the fringe on
    // its edge is the whole point — the channels have to disagree there or the
    // offsets never happened.
    let pixels = through_effect(
        "function hash(seed)
           local h = seed * u32(747796405) + u32(2891336453)
           local word = ((h >> ((h >> u32(28)) + u32(4))) ~ h) * u32(277803737)
           return f32((word >> u32(22)) ~ word & u32(65535)) / 65535.0
         end

         function fragment(uv, time, resolution, amount, jolt, pulse)
           local off = uv - vec2(0.5, 0.5)
           local radius = length(off)
           local breathe = 1.0 + sin(time * 0.9) * pulse
           local shift = off * radius * amount * 4.0 * breathe
           local band = floor(uv.y * 24.0)
           local tick = floor(time * 12.0)
           local tear = hash(u32(band) * u32(374761393) + u32(tick) * u32(668265263))
           tear = max(tear - 0.93, 0.0) * jolt * 0.6
           local slide = vec2(tear, 0.0)
           local r = texture(uv + shift + slide)
           local g = texture(uv + slide * 0.5)
           local b = texture(uv - shift + slide)
           local rgb = vec3(r.x, g.y, b.z)
           local a = max(r.w, max(g.w, b.w))
           local spread = abs(r.x - b.z)
           rgb = rgb + vec3(spread * 0.35, spread * 0.1, spread * 0.45)
           return vec4(rgb, a)
         end",
        &[("amount", 0.35), ("jolt", 0.0), ("pulse", 0.0)],
        Color::rgba8(255, 255, 255, 255),
    );

    // Inside the square every tap lands on white, so there is nothing to
    // separate and the centre stays neutral.
    let centre = [
        channel(&pixels, 32, 32, 0),
        channel(&pixels, 32, 32, 1),
        channel(&pixels, 32, 32, 2),
    ];
    assert!(
        centre[0] == centre[2],
        "the axis of a lens has no aberration: {centre:?}",
    );

    // On the edge the red tap has left the square while the blue one has not,
    // which is a colour that was never in the picture.
    let fringed = (4..12).any(|x| {
        channel(&pixels, x, 32, 0) != channel(&pixels, x, 32, 2)
            && alpha_at(&pixels, SIZE, x, 32) > 0
    });
    assert!(fringed, "the channels came apart at the edge");
}
