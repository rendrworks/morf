use super::*;

// Section 5 of PLAN_WGSL_COVERAGE.md: the surface that needed the *renderer* to
// decide something before the language could name it.
//
// 5.1 declared textures, 5.2 vertex displacement, 5.3 read-only data blocks.

#[test]
fn a_declared_texture_can_be_sampled_by_name() {
    // 5.1. Before this, `texture(uv)` sampled one implicit thing — the layer
    // underneath — and a shader wanting a mask or a lookup table had no way to
    // ask for one.
    let compiled = compile_bound(
        "function fragment(uv, time, resolution, coverage)
           local masked = texture(mask, uv)
           local tinted = texture(ramp, vec2(masked.r, 0.5))
           return vec4(tinted.rgb, masked.a)
         end",
        ShaderKind::Material,
        Vec::new(),
        vec!["mask".to_owned(), "ramp".to_owned()],
        Vec::new(),
    )
    .expect("this compiles");
    assert!(
        compiled.wgsl.contains("var morf_tex0: texture_2d<f32>"),
        "{}",
        compiled.wgsl
    );
    assert!(
        compiled.wgsl.contains("var morf_tex1: texture_2d<f32>"),
        "{}",
        compiled.wgsl
    );
    assert!(
        compiled
            .wgsl
            .contains("textureSample(morf_tex0, morf_tex_sampler0,"),
        "{}",
        compiled.wgsl
    );
    assert_eq!(compiled.textures, vec!["mask", "ramp"]);
    // Sampling a declared texture does not make the node a layer: there is
    // nothing underneath being read.
    assert!(!compiled.samples_behind, "a named texture is not the layer");
}

#[test]
fn sampling_a_texture_that_was_not_declared_says_so() {
    let found = errors(
        "function fragment(uv)
           return texture(missing, uv)
         end",
    );
    assert!(mentions(&found, "not defined here"), "{found:?}");
}

#[test]
fn a_texture_is_not_a_value() {
    let found = compile_bound(
        "function fragment(uv, time, resolution, coverage)
           return vec4(mask.x, 0.0, 0.0, 1.0)
         end",
        ShaderKind::Material,
        Vec::new(),
        vec!["mask".to_owned()],
        Vec::new(),
    )
    .expect_err("a texture cannot be read as a value");
    assert!(mentions(&found, "not a value"), "{found:?}");
    assert!(mentions(&found, "texture(name, uv)"), "{found:?}");
}

#[test]
fn a_data_block_is_read_by_index() {
    // 5.3. Larger than a uniform can hold — a spectrum, a lookup table, a
    // history — and read-only on purpose.
    let compiled = compile_bound(
        "function fragment(uv, time, resolution, coverage)
           local band = i32(uv.x * 64.0)
           local level = spectrum[band]
           return vec4(level, level * 0.5, 1.0 - level, 1.0)
         end",
        ShaderKind::Material,
        Vec::new(),
        Vec::new(),
        vec![("spectrum".to_owned(), Type::F32, 64)],
    )
    .expect("this compiles");
    assert!(
        compiled
            .wgsl
            .contains("var<storage, read> morf_data0: array<f32>"),
        "{}",
        compiled.wgsl
    );
    assert!(
        compiled.wgsl.contains("morf_data0["),
        "and it is indexed: {}",
        compiled.wgsl
    );
    assert_eq!(compiled.data, vec![("spectrum".to_owned(), 64)]);
}

#[test]
fn a_vertex_shader_displaces_a_corner() {
    // 5.2. It moves the *quad*, not the shape inside it: the fragment stage
    // still walks the field in the node's own space, so a displaced node keeps
    // its geometry and takes it somewhere else.
    let compiled = compile_vertex(
        "function vertex(corner, size, time)
           local wave = sin(time + corner.x * 0.05) * 6.0
           return corner + vec2(0.0, wave)
         end",
    )
    .expect("this compiles");
    assert!(
        compiled
            .wgsl
            .contains("fn morf_shader_main(\n    corner: vec2<f32>,"),
        "{}",
        compiled.wgsl
    );
    assert!(compiled.wgsl.contains("-> vec2<f32>"), "{}", compiled.wgsl);
    assert!(compiled.reads_time, "it animates");
}

#[test]
fn a_vertex_shader_cannot_take_a_derivative() {
    // It runs once per corner, before there is a fragment — so there are no
    // neighbouring pixels to differentiate against.
    let found = compile_vertex(
        "function vertex(corner, size, time)
           return corner + vec2(fwidth(corner.x), 0.0)
         end",
    )
    .expect_err("a derivative in a vertex shader is refused");
    assert!(mentions(&found, "cannot take a derivative"), "{found:?}");
    assert!(mentions(&found, "once per corner"), "{found:?}");
}

#[test]
fn a_vertex_shader_returns_a_position_not_a_colour() {
    let found = compile_vertex(
        "function vertex(corner, size, time)
           return vec4(1.0, 0.0, 0.0, 1.0)
         end",
    )
    .expect_err("a colour is not a position");
    assert!(mentions(&found, "vec2"), "{found:?}");
}

#[test]
fn textures_and_data_both_reach_a_helper() {
    // Both are declared at module scope in the generated WGSL, so a helper can
    // name them without being handed them — which is what makes them usable at
    // all in a shader of any size.
    let compiled = compile_bound(
        "function tinted(at)
           return texture(ramp, at) * spectrum[0]
         end

         function fragment(uv, time, resolution, coverage)
           return tinted(uv)
         end",
        ShaderKind::Material,
        Vec::new(),
        vec!["ramp".to_owned()],
        vec![("spectrum".to_owned(), Type::F32, 8)],
    )
    .expect("this compiles");
    assert!(
        compiled.wgsl.contains("fn morf_fn_tinted_0("),
        "{}",
        compiled.wgsl
    );
    assert!(compiled.wgsl.contains("morf_tex0"), "{}", compiled.wgsl);
    assert!(compiled.wgsl.contains("morf_data0"), "{}", compiled.wgsl);
}
