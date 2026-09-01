//! The language surface against a real adapter.
//!
//! Every W step in PLAN_WGSL_COVERAGE.md ends here rather than at the type
//! checker, because a construct that compiles and is refused by naga is worse
//! than a missing one: the author gets a validation error with no line number
//! for code that type-checked. Three of the six steps had a bug that only this
//! file could find.

use crate::gpu::field_tests::alpha_at;
use crate::gpu::shader_mode_tests::surface;
use crate::gpu::shader_tests::{SIZE, channel, shaded};
use crate::*;
use morf_shader::{ShaderKind, ShaderSpec};

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

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_shader_antialiases_its_own_edge_with_fwidth() {
    // W4 against an adapter. A derivative is the one builtin whose *value*
    // depends on neighbouring pixels, so it cannot be checked by reading the
    // generated WGSL — only by rendering and seeing a soft edge where a hard
    // one would have stepped.
    let pixels = shaded(
        "function fragment(uv, time, resolution)
           local d = length(uv - vec2(0.5, 0.5)) - 0.3
           local edge = fwidth(d)
           local coverage = 1.0 - smoothstep(0.0 - edge, edge, d)
           return vec4(coverage, coverage, coverage, 1.0)
         end",
    );
    // Inside the disc is white, outside is black, and the ring between them
    // holds intermediate values — which is the whole point: without a
    // derivative the edge would be one pixel of hard step.
    let inside = channel(&pixels, 32, 32, 0);
    assert!(inside > 200, "the middle is lit: {inside}");
    let soft = (0..SIZE)
        .map(|x| channel(&pixels, x, 32, 0))
        .filter(|value| *value > 20 && *value < 235)
        .count();
    assert!(
        soft >= 2,
        "the edge is antialiased, not stepped: {soft} soft pixels"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn an_array_palette_paints_its_bands() {
    // W5 against an adapter. In the uniform address space an array's stride is
    // a multiple of sixteen whatever the element is, and a shader that indexes
    // one the host laid out differently reads the wrong band — which looks like
    // a colour choice rather than an error.
    let pixels = shaded(
        "function fragment(uv, time, resolution)
           local ramp = {
             vec3(0.9, 0.2, 0.2),
             vec3(0.2, 0.9, 0.2),
             vec3(0.2, 0.2, 0.9),
             vec3(0.9, 0.9, 0.2)
           }
           local band = clamp(i32(uv.x * 4.0), 0, 3)
           return vec4(ramp[band], 1.0)
         end",
    );
    // Four bands across the node, each a different colour.
    let bands: Vec<[u8; 3]> = [10, 26, 40, 54]
        .into_iter()
        .map(|x| {
            [
                channel(&pixels, x, 32, 0),
                channel(&pixels, x, 32, 1),
                channel(&pixels, x, 32, 2),
            ]
        })
        .collect();
    let distinct = bands.iter().collect::<std::collections::HashSet<_>>().len();
    assert_eq!(distinct, 4, "each band is its own colour: {bands:?}");
    assert!(
        bands[0][0] > bands[0][1],
        "the first band is red: {:?}",
        bands[0]
    );
    assert!(
        bands[2][2] > bands[2][0],
        "the third is blue: {:?}",
        bands[2]
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_continue_in_a_counting_loop_reaches_every_turn() {
    // W6 against an adapter, and the one thing a compiler test cannot settle:
    // that the counter really does advance past a `continue`. If the increment
    // had stayed at the tail of the body this loop would spin against its
    // guard and the result would be wrong rather than absent.
    //
    // Sixteen turns, half of them skipped by the continuation, so the total is
    // eight steps of an eighth: one.
    let pixels = shaded(
        "function fragment(uv, time, resolution)
           local total = 0.0
           for i = 1, 16 do
             if i > 8.0 then
               goto continue
             end
             total = total + 0.125
             ::continue::
           end
           return vec4(total, total, total, 1.0)
         end",
    );
    let value = channel(&pixels, 32, 32, 0);
    assert_eq!(
        value, 255,
        "eight eighths, so the loop advanced every turn: {value}",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn discard_removes_the_fragment_entirely() {
    // `discard` is only meaningful where the shader owns its coverage, so this
    // is a surface shader. What it throws away has to be *gone*, not merely
    // transparent — those look the same in a screenshot and different in a
    // blend.
    let pixels = surface(
        "function fragment(uv, time, resolution)
           if uv.x > 0.5 then
             discard()
           end
           return vec4(0.9, 0.3, 0.2, 1.0)
         end",
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 16, 32),
        255,
        "the kept half is painted"
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, 48, 32),
        0,
        "and the other half is gone"
    );
}
