//! Shaders through the real paint path, rather than a hand-built draw list.
//!
//! Every other GPU test here constructs its `DrawList` directly, which is right
//! for asking whether a generated shader is WGSL a driver accepts — and blind
//! to whether a configuration can actually reach that code. It cannot, if the
//! effect ends up on a node whose subtree is empty: the shader compiles, the
//! layer is created, the pipeline runs, and it samples a transparent target and
//! returns nothing. These tests start from a scene and a layout, so the answer
//! they give is the one a configuration would get.

use crate::gpu::backend_types::ShaderRegistration;
use crate::gpu::field_tests::read_frame;
use crate::gpu::shader_tests::{SIZE, channel};
use crate::tests::NoText;
use crate::*;
use morf_layout::{Layout, Size};
use morf_scene::{Element, NodeShader, Scene};
use morf_shader::{ShaderKind, ShaderSpec};

/// Compiles the channel-swapping effect and registers it on a backend.
///
/// Swapping red for blue is a change nothing but a real sample could produce,
/// which is what makes it worth asserting on.
fn swapper(backend: &mut WgpuBackend) -> u64 {
    let spec = ShaderSpec {
        kind: ShaderKind::Effect,
        inputs: ShaderSpec::default_inputs(ShaderKind::Effect),
        params: Vec::new(),
        entry: "fragment".to_owned(),
        textures: Vec::new(),
        data: Vec::new(),
        vertex: false,
    };
    let compiled = morf_shader::compile(
        "function fragment(uv)
           local under = texture(uv)
           return vec4(under.b, under.g, under.r, under.a)
         end",
        &spec,
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("effect", &errors)));
    backend
        .register_shader(ShaderRegistration {
            program: compiled.hash,
            wgsl: Some(&compiled.wgsl),
            vertex: None,
            offsets: &[],
            uniform_size: compiled.uniform_size,
            owns_coverage: false,
            effect: true,
            textures: &[],
            data: &[],
        })
        .expect("the generated WGSL compiles");
    compiled.hash
}

/// Paints a red square, with the effect either wrapping it or beside it.
fn painted(backend: &mut WgpuBackend, program: u64, wrapping: bool) -> Vec<u8> {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let carrier = scene.create(Element::Item);
    let square = scene.create(Element::Rect);
    scene.assign(square, "width", SIZE as f64).unwrap();
    scene.assign(square, "height", SIZE as f64).unwrap();
    scene.assign(square, "color", "#ff0000ff").unwrap();
    scene.assign(carrier, "width", SIZE as f64).unwrap();
    scene.assign(carrier, "height", SIZE as f64).unwrap();
    scene.attach_shader(
        carrier,
        NodeShader {
            program,
            params: Vec::new(),
            data: Vec::new(),
            samples_behind: true,
            owns_coverage: false,
        },
    );
    // The whole difference between the two cases: whether the square is inside
    // the node carrying the effect, or merely underneath it.
    scene
        .reparent(square, Some(if wrapping { carrier } else { root }))
        .unwrap();
    scene.reparent(carrier, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: SIZE as f64,
            height: SIZE as f64,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    read_frame(backend, &list, SIZE)
}

#[test]
#[ignore = "requires a GPU adapter"]
fn an_effect_wrapping_its_content_reworks_it() {
    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    let program = swapper(&mut backend);
    let pixels = painted(&mut backend, program, true);
    assert_eq!(
        channel(&pixels, 32, 32, 2),
        255,
        "the red square arrived as blue",
    );
    assert_eq!(channel(&pixels, 32, 32, 0), 0, "and nothing stayed red");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn an_effect_beside_its_content_leaves_it_alone() {
    // Stated as a test because it is the mistake anyone writes first, and
    // because it fails silently: the shader is correct, the pipeline runs, and
    // the picture comes out exactly as though no shader had been asked for.
    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    let program = swapper(&mut backend);
    let pixels = painted(&mut backend, program, false);
    assert_eq!(
        channel(&pixels, 32, 32, 0),
        255,
        "the square is untouched: an effect samples its own subtree, and a \
         sibling's subtree does not contain it",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_material_shader_on_a_field_sizes_itself_from_its_own_derivatives() {
    // What `examples/sdf-blobs-crt.lua` relies on. A material shader is not
    // told how large the node it is colouring is — `resolution` is the surface,
    // not the node — so the per-blob tube takes its own width in pixels from
    // the rate `uv` changes per pixel. That has to survive the real paint path,
    // because a field's `uv` is set up there and nowhere else: get it wrong and
    // every blob draws the same number of scanlines regardless of its size.
    let spec = ShaderSpec {
        kind: ShaderKind::Material,
        inputs: ShaderSpec::default_inputs(ShaderKind::Material),
        params: Vec::new(),
        entry: "fragment".to_owned(),
        textures: Vec::new(),
        data: Vec::new(),
        vertex: false,
    };
    let compiled = morf_shader::compile(
        "function fragment(uv, time, resolution, coverage)
           -- The node's own width in pixels, and a stripe every three of them.
           local across = 1.0 / max(fwidth(uv.x), 0.000001)
           local stripe = i32(floor(uv.x * across)) % 3
           local lit = 0.0
           if stripe == 0 then
             lit = 1.0
           end
           return vec4(lit, lit, lit, 1.0)
         end",
        &spec,
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("material", &errors)));

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    backend
        .register_shader(ShaderRegistration {
            program: compiled.hash,
            wgsl: Some(&compiled.wgsl),
            vertex: None,
            offsets: &[],
            uniform_size: compiled.uniform_size,
            owns_coverage: false,
            effect: false,
            textures: &[],
            data: &[],
        })
        .expect("the generated WGSL compiles");

    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let field = scene.create(Element::Sdf);
    let circle = scene.create(Element::SdfShape);
    for node in [field, circle] {
        scene.assign(node, "width", SIZE as f64).unwrap();
        scene.assign(node, "height", SIZE as f64).unwrap();
    }
    scene.assign(circle, "shape", "circle").unwrap();
    scene.assign(field, "fill_color", "#ffffffff").unwrap();
    scene.attach_shader(
        field,
        NodeShader {
            program: compiled.hash,
            params: Vec::new(),
            data: Vec::new(),
            samples_behind: false,
            owns_coverage: false,
        },
    );
    scene.reparent(circle, Some(field)).unwrap();
    scene.reparent(field, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: SIZE as f64,
            height: SIZE as f64,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let pixels = read_frame(&mut backend, &list, SIZE);

    // Across the middle of the circle the stripes have to alternate. One flat
    // value would mean `fwidth` came back as zero or as something unrelated to
    // the node, and the shader would have quietly stopped measuring anything.
    let row: Vec<u8> = (24..40).map(|x| channel(&pixels, x, 32, 1)).collect();
    assert!(
        row.iter().any(|v| *v > 128) && row.iter().any(|v| *v < 128),
        "the stripes alternate across the node: {row:?}",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn an_effect_shader_can_carry_a_data_block() {
    // What `examples/sdf-blobs-chroma.lua` needs: an effect that is told where
    // the blobs are, so it can pick its axis per pixel.
    //
    // Effect shaders are built on the glyph pipeline rather than the field one,
    // and that builder was only ever given two bind group layouts. The compiler
    // emits the same group numbers whatever mode a shader is in — three for a
    // data block — so declaring one produced WGSL naming a group the pipeline
    // layout did not have, and wgpu refused the pipeline outright. That is a
    // panic at load, not a wrong picture, so it needs a test that gets as far
    // as building the thing.
    let spec = ShaderSpec {
        kind: ShaderKind::Effect,
        inputs: ShaderSpec::default_inputs(ShaderKind::Effect),
        params: Vec::new(),
        entry: "fragment".to_owned(),
        textures: Vec::new(),
        data: vec![("places".to_owned(), morf_shader::Type::F32, 4)],
        vertex: false,
    };
    let compiled = morf_shader::compile(
        "function fragment(uv, time, resolution)
           local under = texture(uv)
           -- A computed index, so the block is genuinely read rather than
           -- declared and quietly ignored.
           local slot = clamp(i32(uv.x * 4.0), 0, 3)
           return vec4(under.r * places[slot], under.g, under.b, under.a)
         end",
        &spec,
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("effect", &errors)));

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    backend
        .register_shader(ShaderRegistration {
            program: compiled.hash,
            wgsl: Some(&compiled.wgsl),
            vertex: None,
            offsets: &[],
            uniform_size: compiled.uniform_size,
            owns_coverage: false,
            effect: true,
            textures: &[],
            data: &[("places".to_owned(), 4)],
        })
        .expect("the effect pipeline is built with a group for its data block");

    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let carrier = scene.create(Element::Item);
    let square = scene.create(Element::Rect);
    scene.assign(square, "width", SIZE as f64).unwrap();
    scene.assign(square, "height", SIZE as f64).unwrap();
    scene.assign(square, "color", "#ffffffff").unwrap();
    scene.assign(carrier, "width", SIZE as f64).unwrap();
    scene.assign(carrier, "height", SIZE as f64).unwrap();
    scene.attach_shader(
        carrier,
        NodeShader {
            program: compiled.hash,
            params: Vec::new(),
            // Dark on the left, bright on the right.
            data: vec![vec![0.0, 0.33, 0.66, 1.0]],
            samples_behind: true,
            owns_coverage: false,
        },
    );
    scene.reparent(square, Some(carrier)).unwrap();
    scene.reparent(carrier, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: SIZE as f64,
            height: SIZE as f64,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let pixels = read_frame(&mut backend, &list, SIZE);

    let left = channel(&pixels, 4, 32, 0);
    let right = channel(&pixels, 60, 32, 0);
    assert!(
        right > left + 100,
        "the block's values reached the effect in order: {left} then {right}",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_shift_stated_in_pixels_moves_by_that_many_pixels() {
    // `examples/sdf-blobs-chroma.lua` states its aberration in pixels, because
    // a fringe written as a fraction of the surface is far too easy to ask for
    // an enormous amount of by accident — and once the shift is wide enough to
    // reach a neighbouring blob it samples that blob's colour, which reads as
    // the blobs bleeding into one another rather than as glass.
    //
    // Converting pixels back to `uv` is where that goes quietly wrong, and it
    // goes wrong by the aspect ratio, so it needs checking on both axes. A
    // square is sampled eight pixels to the right, which moves its content
    // eight pixels left, and the edge is measured to say so.
    const SHIFT: usize = 8;
    let spec = ShaderSpec {
        kind: ShaderKind::Effect,
        inputs: ShaderSpec::default_inputs(ShaderKind::Effect),
        params: Vec::new(),
        entry: "fragment".to_owned(),
        textures: Vec::new(),
        data: Vec::new(),
        vertex: false,
    };
    let compiled = morf_shader::compile(
        "function fragment(uv, time, resolution)
           return texture(uv + vec2(8.0 / resolution.x, 0.0))
         end",
        &spec,
    )
    .unwrap_or_else(|errors| panic!("{}", morf_shader::report("effect", &errors)));

    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    backend
        .register_shader(ShaderRegistration {
            program: compiled.hash,
            wgsl: Some(&compiled.wgsl),
            vertex: None,
            offsets: &[],
            uniform_size: compiled.uniform_size,
            owns_coverage: false,
            effect: true,
            textures: &[],
            data: &[],
        })
        .expect("the generated WGSL compiles");

    // A square inset far enough that a shifted edge is still inside the target.
    const INSET: usize = 20;
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let carrier = scene.create(Element::Item);
    // A layer's bounds are the union of what it holds, and an effect's `uv`
    // spans those bounds while `resolution` is the surface — so the two only
    // agree when the layer covers the surface. This transparent rectangle is
    // what makes them agree, and it is why the examples carry one: without it
    // a shift stated in pixels is measured against a box that changes size as
    // the content moves.
    let pin = scene.create(Element::Rect);
    scene.assign(pin, "width", SIZE as f64).unwrap();
    scene.assign(pin, "height", SIZE as f64).unwrap();
    scene.assign(pin, "color", "#00000000").unwrap();
    let square = scene.create(Element::Rect);
    scene.assign(square, "x", INSET as f64).unwrap();
    scene
        .assign(square, "width", (SIZE as usize - INSET * 2) as f64)
        .unwrap();
    scene.assign(square, "height", SIZE as f64).unwrap();
    scene.assign(square, "color", "#ffffffff").unwrap();
    scene.assign(carrier, "width", SIZE as f64).unwrap();
    scene.assign(carrier, "height", SIZE as f64).unwrap();
    scene.attach_shader(
        carrier,
        NodeShader {
            program: compiled.hash,
            params: Vec::new(),
            data: Vec::new(),
            samples_behind: true,
            owns_coverage: false,
        },
    );
    scene.reparent(pin, Some(carrier)).unwrap();
    scene.reparent(square, Some(carrier)).unwrap();
    scene.reparent(carrier, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: SIZE as f64,
            height: SIZE as f64,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let pixels = read_frame(&mut backend, &list, SIZE);

    // The first lit column along the middle row.
    let edge = (0..SIZE as usize)
        .find(|x| channel(&pixels, *x as u32, 32, 1) > 128)
        .expect("the square is somewhere");
    assert_eq!(
        edge,
        INSET - SHIFT,
        "eight pixels asked for is eight pixels moved, not eight of something else",
    );
}
