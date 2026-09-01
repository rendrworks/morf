use crate::gpu::field_tests::{alpha_at, field_command, field_layer, read_frame};
use crate::*;
use morf_layout::Geometry;
use morf_scene::Color;
use morf_shader::{ShaderKind, ShaderSpec};

// A configuration's own shader, compiled from Lua and painted on a GPU.
//
// These are the tests that say the whole path works: Luna's parser, the type
// checker, the WGSL printer, the pipeline splice, and the uniform block, end to
// end against a real adapter.

const SIZE: u32 = 64;

fn compile(body: &str) -> morf_shader::Compiled {
    let spec = ShaderSpec {
        kind: ShaderKind::Material,
        inputs: ShaderSpec::default_inputs(ShaderKind::Material),
        params: Vec::new(),
        entry: "fragment".to_owned(),
    };
    morf_shader::compile(body, &spec)
        .unwrap_or_else(|errors| panic!("{}", morf_shader::report("test", &errors)))
}

/// Renders one shaded node and hands back the pixels.
fn shaded(body: &str) -> Vec<u8> {
    let compiled = compile(body);
    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    let offsets: Vec<u32> = compiled.params.iter().map(|slot| slot.offset).collect();
    backend
        .register_shader(
            compiled.hash,
            &compiled.wgsl,
            &offsets,
            compiled.uniform_size,
            false,
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
            owns_coverage: false,
        });
    }
    let list = DrawList {
        commands: vec![command],
        layers: Vec::new(),
    };
    read_frame(&mut backend, &list, SIZE)
}

fn channel(pixels: &[u8], x: u32, y: u32, index: usize) -> u8 {
    pixels[((y * SIZE + x) * 4) as usize + index]
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_shader_paints_inside_the_shape_and_nowhere_else() {
    // The seam: the field still decides coverage, the shader only decides
    // colour. A shader that painted outside its node would mean the geometry
    // path had been bypassed, which is the whole thing this mode promises not
    // to do.
    let pixels = shaded(
        "function fragment(uv)
           return vec4(1.0, 0.0, 0.0, 1.0)
         end",
    );
    assert_eq!(alpha_at(&pixels, SIZE, 32, 32), 255, "the shape is painted");
    assert_eq!(
        channel(&pixels, 32, 32, 0),
        255,
        "and it is the shader's red"
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 2, 2),
        0,
        "and nothing outside the shape was touched",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn uv_runs_left_to_right_and_top_to_bottom() {
    // The orientation test. A flipped `uv` is the failure that looks almost
    // right and is silently upside-down in every shader anyone writes.
    let pixels = shaded(
        "function fragment(uv)
           return vec4(uv.x, uv.y, 0.0, 1.0)
         end",
    );
    let left = channel(&pixels, 12, 32, 0);
    let right = channel(&pixels, 52, 32, 0);
    assert!(
        right > left + 64,
        "red rises to the right: {left} then {right}"
    );
    let top = channel(&pixels, 32, 12, 1);
    let bottom = channel(&pixels, 32, 52, 1);
    assert!(
        bottom > top + 64,
        "green rises downwards: {top} then {bottom}",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_loop_with_no_exit_still_finishes_the_frame() {
    // The most important test in the crate. `while true do end` is what takes a
    // compositor down — the device is lost, and with it the bar, the lock
    // screen and the session. The emitted guard is the only reason this
    // returns at all, and if it ever stops working this test hangs rather than
    // failing, which is itself the signal.
    let pixels = shaded(
        "function fragment(uv)
           local total = 0.0
           while true do
             total = total + 0.0001
           end
           return vec4(total, 0.0, 0.0, 1.0)
         end",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 32),
        255,
        "the frame completed and the shape is still painted",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn arithmetic_and_builtins_produce_the_values_they_should() {
    // A shader that computes something checkable by hand, so a wrong operator
    // or a mis-emitted builtin shows up as a number rather than as "looks odd".
    let pixels = shaded(
        "function fragment(uv)
           local d = clamp(uv.x * 2.0 - 0.5, 0.0, 1.0)
           return vec4(d, sqrt(0.25), 0.0, 1.0)
         end",
    );
    // Green is sqrt(0.25) = 0.5 everywhere inside the shape — and the target is
    // sRGB, so a linear half comes back as 188, not 128. Zero and one are the
    // transfer's fixed points, which is why the red assertions below need no
    // such adjustment.
    let green = channel(&pixels, 32, 32, 1);
    assert!(green.abs_diff(188) <= 2, "sqrt(0.25) is a half: {green}");
    // Red is clamped to zero on the left and to one on the right. Both samples
    // stay inside the shape, which spans eight to fifty-six: outside it the
    // field's own coverage is zero and the shader's colour never appears.
    assert_eq!(channel(&pixels, 10, 32, 0), 0, "clamped low on the left");
    assert_eq!(
        channel(&pixels, 50, 32, 0),
        255,
        "clamped high on the right"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_branch_picks_different_colours_across_the_node() {
    let pixels = shaded(
        "function fragment(uv)
           if uv.x > 0.5 then
             return vec4(0.0, 0.0, 1.0, 1.0)
           else
             return vec4(1.0, 0.0, 0.0, 1.0)
           end
         end",
    );
    assert_eq!(channel(&pixels, 14, 32, 0), 255, "red on the left");
    assert_eq!(channel(&pixels, 50, 32, 2), 255, "blue on the right");
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_parameter_reaches_the_shader() {
    let spec = ShaderSpec {
        kind: ShaderKind::Material,
        inputs: ShaderSpec::default_inputs(ShaderKind::Material),
        params: vec![morf_shader::Binding {
            name: "level".to_owned(),
            ty: morf_shader::Type::F32,
        }],
        entry: "fragment".to_owned(),
    };
    let compiled = morf_shader::compile(
        "function fragment(uv, time, resolution, coverage, level)
           return vec4(level, 0.0, 0.0, 1.0)
         end",
        &spec,
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("test", &errors)));

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    let offsets: Vec<u32> = compiled.params.iter().map(|slot| slot.offset).collect();
    backend
        .register_shader(
            compiled.hash,
            &compiled.wgsl,
            &offsets,
            compiled.uniform_size,
            false,
            false,
        )
        .expect("compiles");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut command = field_command(node, vec![field_layer(8.0, 8.0, 48.0, Shape::Box)]);
    if let DrawCommand::Field { shader, .. } = &mut command {
        *shader = Some(ShaderBinding {
            program: compiled.hash,
            params: vec![0.5],
            samples_behind: false,
            owns_coverage: false,
        });
    }
    let list = DrawList {
        commands: vec![command],
        layers: Vec::new(),
    };
    let pixels = read_frame(&mut backend, &list, SIZE);
    let red = channel(&pixels, 32, 32, 0);
    assert!(
        red.abs_diff(188) <= 2,
        "the parameter arrived at its declared offset: {red}",
    );
}

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
