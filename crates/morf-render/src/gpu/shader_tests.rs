use crate::gpu::field_tests::{alpha_at, field_command, field_layer, read_frame};
use crate::*;
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
        )
        .expect("the generated WGSL compiles");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut command = field_command(node, vec![field_layer(8.0, 8.0, 48.0, Shape::Box)]);
    if let DrawCommand::Field { shader, .. } = &mut command {
        *shader = Some(ShaderBinding {
            program: compiled.hash,
            params: Vec::new(),
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
        )
        .expect("compiles");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut command = field_command(node, vec![field_layer(8.0, 8.0, 48.0, Shape::Box)]);
    if let DrawCommand::Field { shader, .. } = &mut command {
        *shader = Some(ShaderBinding {
            program: compiled.hash,
            params: vec![0.5],
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
