//! Section 5 of PLAN_WGSL_COVERAGE.md against an adapter.
//!
//! These are the features that needed the *renderer* to decide something first:
//! a declared texture is a bind group the host builds, a data block is a buffer
//! it fills, and a vertex displacement is a second stage spliced into the same
//! module. None of them can be checked by reading generated WGSL.

use crate::gpu::backend_types::ShaderRegistration;
use crate::gpu::field_tests::alpha_at;
use crate::gpu::shader_tests::{SIZE, channel, shaded};
use crate::*;
use morf_shader::{ShaderKind, ShaderSpec};

/// Compiles and renders a shader with declared textures and data blocks.
fn bound(
    body: &str,
    textures: &[(&str, &str)],
    data: &[(&str, u32)],
    values: &[Vec<f32>],
) -> Vec<u8> {
    let spec = ShaderSpec {
        kind: ShaderKind::Material,
        inputs: ShaderSpec::default_inputs(ShaderKind::Material),
        params: Vec::new(),
        textures: textures
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect(),
        data: data
            .iter()
            .map(|(name, length)| ((*name).to_owned(), morf_shader::Type::F32, *length))
            .collect(),
        entry: "fragment".to_owned(),
        vertex: false,
    };
    let compiled = morf_shader::compile(body, &spec)
        .unwrap_or_else(|errors| panic!("{}", morf_shader::report("test", &errors)));

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    let paths: Vec<String> = textures
        .iter()
        .map(|(_, path)| (*path).to_owned())
        .collect();
    let blocks: Vec<(String, u32)> = data
        .iter()
        .map(|(name, length)| ((*name).to_owned(), *length))
        .collect();
    backend
        .register_shader(ShaderRegistration {
            program: compiled.hash,
            wgsl: Some(&compiled.wgsl),
            vertex: None,
            offsets: &[],
            uniform_size: compiled.uniform_size,
            owns_coverage: false,
            effect: false,
            textures: &paths,
            data: &blocks,
        })
        .expect("the generated WGSL compiles and the textures load");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut command = crate::gpu::field_tests::field_command(
        node,
        vec![crate::gpu::field_tests::field_layer(
            4.0,
            4.0,
            56.0,
            Shape::Box,
        )],
    );
    if let DrawCommand::Field { shader, .. } = &mut command {
        *shader = Some(ShaderBinding {
            program: compiled.hash,
            params: Vec::new(),
            data: values.to_vec(),
            samples_behind: false,
            owns_coverage: false,
        });
    }
    let list = DrawList {
        commands: vec![command],
        layers: Vec::new(),
    };
    crate::gpu::field_tests::read_frame(&mut backend, &list, SIZE)
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_data_block_reaches_the_shader() {
    // §5.3. A read-only block the host fills each frame — larger than a uniform
    // can hold, and read-only because every pixel of the node runs this shader
    // and a writable one would be a race between all of them.
    let levels: Vec<f32> = (0..8).map(|index| index as f32 / 7.0).collect();
    let pixels = bound(
        "function fragment(uv, time, resolution, coverage)
           local band = clamp(i32(uv.x * 8.0), 0, 7)
           local level = spectrum[band]
           return vec4(level, level, level, 1.0)
         end",
        &[],
        &[("spectrum", 8)],
        &[levels],
    );
    assert_eq!(alpha_at(&pixels, SIZE, 32, 32), 255, "the node painted");
    // The block ramps from zero to one, so the node does too.
    let left = channel(&pixels, 8, 32, 0);
    let right = channel(&pixels, 56, 32, 0);
    assert!(
        right > left + 100,
        "the block's values arrived in order: {left} then {right}",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_vertex_shader_moves_the_quad() {
    // §5.2. It moves the quad, not the shape inside it — so a node displaced
    // sideways paints where it was sent, and nowhere near where it started.
    let fragment = morf_shader::compile(
        "function fragment(uv, time, resolution, coverage)
           return vec4(1.0, 0.4, 0.2, 1.0)
         end",
        &ShaderSpec {
            kind: ShaderKind::Material,
            inputs: ShaderSpec::default_inputs(ShaderKind::Material),
            params: Vec::new(),
            textures: Vec::new(),
            data: Vec::new(),
            entry: "fragment".to_owned(),
            vertex: false,
        },
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("fragment", &errors)));
    let displace = morf_shader::compile(
        "function vertex(corner, size, time)
           -- A fixed shove downwards, so the test does not depend on the clock.
           return corner + vec2(0.0, 18.0)
         end",
        &ShaderSpec {
            kind: ShaderKind::Material,
            inputs: ShaderSpec::vertex_inputs(),
            params: Vec::new(),
            textures: Vec::new(),
            data: Vec::new(),
            entry: "vertex".to_owned(),
            vertex: true,
        },
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("vertex", &errors)));

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    backend
        .register_shader(ShaderRegistration {
            program: fragment.hash,
            wgsl: Some(&fragment.wgsl),
            vertex: Some(&displace.wgsl),
            offsets: &[],
            uniform_size: fragment.uniform_size,
            owns_coverage: false,
            effect: false,
            textures: &[],
            data: &[],
        })
        .expect("both stages compile together");

    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut command = crate::gpu::field_tests::field_command(
        node,
        vec![crate::gpu::field_tests::field_layer(
            8.0,
            4.0,
            20.0,
            Shape::Box,
        )],
    );
    if let DrawCommand::Field { shader, .. } = &mut command {
        *shader = Some(ShaderBinding {
            program: fragment.hash,
            params: Vec::new(),
            data: Vec::new(),
            samples_behind: false,
            owns_coverage: false,
        });
    }
    let list = DrawList {
        commands: vec![command],
        layers: Vec::new(),
    };
    let pixels = crate::gpu::field_tests::read_frame(&mut backend, &list, SIZE);

    // The layer sits at y 4..24 and the shove is eighteen pixels down, so it
    // paints around y 22..42 and nothing is left at the top.
    assert_eq!(
        alpha_at(&pixels, SIZE, 18, 32),
        255,
        "it painted where it was sent"
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 18, 8),
        0,
        "and not where it started"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_switch_and_an_exact_texel_read_reach_the_driver() {
    // The last two boxes, through an adapter. A `switch` naga rejects — a
    // duplicate case, a missing default, a case constant of the wrong width —
    // is refused with no line number, and an emitter that *generates* the
    // construct is exactly where that would come from.
    let pixels = shaded(
        "function fragment(uv, time, resolution, coverage)
           local band = clamp(i32(uv.x * 4.0), 0, 3)
           local shade = 0.0
           if band == 0 then
             shade = 0.15
           elseif band == 1 then
             shade = 0.45
           elseif band == 2 then
             shade = 0.75
           else
             shade = 1.0
           end
           return vec4(shade, shade * 0.5, 1.0 - shade, 1.0)
         end",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 32),
        255,
        "the driver accepted it"
    );
    // Four bands, each its own shade, in order across the node.
    // All four inside the shape, which spans eight to fifty-six: outside it
    // the field's coverage is zero and the shader's colour never appears.
    let shades: Vec<u8> = [10, 24, 38, 50]
        .into_iter()
        .map(|x| channel(&pixels, x, 32, 0))
        .collect();
    assert!(
        shades.windows(2).all(|pair| pair[1] > pair[0]),
        "the switch picked each band in turn: {shades:?}",
    );
}
