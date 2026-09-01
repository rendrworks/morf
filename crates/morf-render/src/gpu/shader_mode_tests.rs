//! Surface and effect shaders, and the language surface, against an adapter.
//!
//! Split from `shader_tests` when the two together crossed the line gate. The
//! ones here need something the material tests do not: their own coverage,
//! something rendered underneath them, or a builtin whose emission a driver has
//! never seen.

use crate::gpu::field_tests::{alpha_at, field_command, field_layer, read_frame};
use crate::gpu::shader_tests::{SIZE, channel, shaded};
use crate::*;
use morf_layout::Geometry;
use morf_scene::Color;
use morf_shader::{ShaderKind, ShaderSpec};

/// Renders one surface-mode shader, where the shader decides its own coverage.
fn surface(body: &str) -> Vec<u8> {
    let spec = ShaderSpec {
        kind: ShaderKind::Surface,
        inputs: ShaderSpec::default_inputs(ShaderKind::Surface),
        params: Vec::new(),
        entry: "fragment".to_owned(),
    };
    let compiled = morf_shader::compile(body, &spec)
        .unwrap_or_else(|errors| panic!("{}", morf_shader::report("test", &errors)));
    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    backend
        .register_shader(
            compiled.hash,
            &compiled.wgsl,
            &[],
            compiled.uniform_size,
            true,
            false,
        )
        .expect("the generated WGSL compiles");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut command = field_command(node, vec![field_layer(8.0, 8.0, 48.0, Shape::Box)]);
    if let DrawCommand::Field { shader, .. } = &mut command {
        *shader = Some(ShaderBinding {
            program: compiled.hash,
            params: Vec::new(),
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

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn an_effect_shader_reads_what_is_underneath_it() {
    // The mode that needs a layer: until the subtree has been rendered into its
    // own target there is nothing to sample. A red square is drawn, and the
    // effect swaps its channels — so what comes back is blue, which it could
    // only know by reading what was already there.
    let spec = ShaderSpec {
        kind: ShaderKind::Effect,
        inputs: ShaderSpec::default_inputs(ShaderKind::Effect),
        params: Vec::new(),
        entry: "fragment".to_owned(),
    };
    let compiled = morf_shader::compile(
        "function fragment(uv)
           local under = texture(uv)
           return vec4(under.b, under.g, under.r, under.a)
         end",
        &spec,
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("test", &errors)));
    assert!(compiled.samples_behind, "the compiler saw the sample");

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    backend
        .register_shader(
            compiled.hash,
            &compiled.wgsl,
            &[],
            compiled.uniform_size,
            false,
            true,
        )
        .expect("the generated WGSL compiles");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut layer = field_layer(8.0, 8.0, 48.0, Shape::Box);
    layer.color = Color::rgba8(255, 0, 0, 255);
    let mut command = field_command(node, vec![layer]);
    if let DrawCommand::Field { fill_color, .. } = &mut command {
        *fill_color = Color::rgba8(255, 0, 0, 255);
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
                width: 64.0,
                height: 64.0,
            },
            opacity: 1.0,
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_offset: [0.0, 0.0],
            mask: None,
            shader: Some(ShaderBinding {
                program: compiled.hash,
                params: Vec::new(),
                samples_behind: true,
                owns_coverage: false,
            }),
        }],
    };

    let pixels = read_frame(&mut backend, &list, SIZE);

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
pub(crate) fn the_new_builtins_are_accepted_by_a_driver() {
    // A builtin that compiles here and is refused by naga is worse than a
    // missing one: the configuration author gets a validation error with no
    // line number instead of a diagnostic. Every W1 addition goes through an
    // adapter once.
    let pixels = shaded(
        "function fragment(uv, time, resolution)
           local i = vec3(0.3, -0.8, 0.5)
           local n = vec3(0.0, 1.0, 0.0)
           local bent = refract(i, n, 0.66) + faceforward(n, i, n)
           local scalar = saturate(uv.x * 2.0 - 0.5)
             + trunc(uv.y * 3.0) * 0.1
             + inversesqrt(uv.x + 1.0) * 0.2
             + fma(uv.y, 0.25, 0.1)
             + atanh(uv.x * 0.5) * 0.1
             + asinh(uv.y) * 0.1
             + acosh(uv.x + 1.0) * 0.1
             + quantize_to_f16(uv.y) * 0.1
           return vec4(saturate(scalar), saturate(bent.y), 0.4, 1.0)
         end",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 32),
        255,
        "the driver accepted it"
    );
    let left = channel(&pixels, 14, 32, 0);
    let right = channel(&pixels, 50, 32, 0);
    assert_ne!(left, right, "and the arithmetic varies across the node");
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_rotation_matrix_paints_a_turned_shape() {
    // W2 against an adapter. A matrix that compiles and is laid out wrongly is
    // the failure worth catching here: the uniform block's alignment rules are
    // where matrices actually go wrong, and a wrong layout reads back as a
    // shape in the wrong place rather than as an error.
    let pixels = shaded(
        "function fragment(uv, time, resolution)
           local p = uv - vec2(0.5, 0.5)
           -- An eighth turn, written the way anybody would write it.
           local a = 0.7853981
           local turn = mat2(vec2(cos(a), sin(a)), vec2(0.0 - sin(a), cos(a)))
           local q = turn * p
           -- A bar along one axis: if the rotation did nothing, it stays
           -- horizontal, and the corner samples below would not change.
           local bar = 1.0 - step(0.08, abs(q.y))
           return vec4(bar, bar * 0.4, 0.2, 1.0)
         end",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 32),
        255,
        "the bar crosses the middle"
    );
    // Rotated by an eighth turn the bar runs corner to corner, so it covers a
    // point up and to the right of centre that a horizontal bar would miss.
    let turned = channel(&pixels, 44, 20, 0);
    assert!(turned > 128, "the bar was actually turned: {turned}");
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_matrix_parameter_arrives_with_its_columns_intact() {
    // The uniform layout rule: a `mat3x3` is three sixteen-byte columns, not
    // nine floats. If the host and the shader disagree, the columns arrive
    // shuffled — which looks like a shear rather than an error.
    let spec = ShaderSpec {
        kind: ShaderKind::Material,
        inputs: ShaderSpec::default_inputs(ShaderKind::Material),
        params: vec![morf_shader::Binding {
            name: "turn".to_owned(),
            ty: morf_shader::Type::Mat3,
        }],
        entry: "fragment".to_owned(),
    };
    let compiled = morf_shader::compile(
        "function fragment(uv, time, resolution, coverage, turn)
           local v = turn * vec3(uv.x, uv.y, 1.0)
           return vec4(v.x, v.y, v.z, 1.0)
         end",
        &spec,
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("test", &errors)));

    // Three columns at sixteen-byte stride, after the frame's own header.
    assert_eq!(compiled.params[0].offset, morf_shader::HEADER_BYTES);
    assert_eq!(
        compiled.uniform_size,
        morf_shader::HEADER_BYTES + 48,
        "three sixteen-byte columns, not nine floats",
    );

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    backend
        .register_shader(
            compiled.hash,
            &compiled.wgsl,
            &[compiled.params[0].offset],
            compiled.uniform_size,
            false,
            false,
        )
        .expect("the generated WGSL compiles");
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn integer_hash_noise_paints() {
    // W3 against an adapter. Integer arithmetic is where a compiler most easily
    // emits something plausible that a driver refuses — a mixed-width shift, a
    // literal that went through a float on the way — so the hash goes through a
    // real pipeline rather than only through the type checker.
    let pixels = shaded(
        "function hash(seed)
           local h = seed * u32(747796405) + u32(2891336453)
           local word = ((h >> ((h >> u32(28)) + u32(4))) ~ h) * u32(277803737)
           return (word >> u32(22)) ~ word
         end

         function fragment(uv, time, resolution)
           local cell = floor(uv * 12.0)
           local seed = u32(cell.x) * u32(374761393) + u32(cell.y) * u32(668265263)
           local noise = f32(hash(seed) & u32(65535)) / 65535.0
           return vec4(noise, noise, noise, 1.0)
         end",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 32),
        255,
        "the driver accepted it"
    );
    // Noise is noise: sampling a spread of cells has to give a spread of
    // values. A hash that collapsed to one number would paint a flat grey and
    // pass every weaker assertion.
    let samples: std::collections::HashSet<u8> = [(8, 8), (20, 14), (32, 30), (44, 40), (54, 52)]
        .into_iter()
        .map(|(x, y)| channel(&pixels, x, y, 0))
        .collect();
    assert!(samples.len() >= 3, "the hash actually varies: {samples:?}");
}
