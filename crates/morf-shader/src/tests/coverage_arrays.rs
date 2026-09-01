use super::*;

// Arrays and indexing. Split out when `coverage_types` crossed the line gate;
// these are the one W step whose subject is a container rather than a number.

/// The WGSL for a body expected to compile, with the shader's own name in the
/// panic when it does not.
fn emitted(name: &str, body: &str) -> String {
    match compile_material(body) {
        Ok(compiled) => compiled.wgsl,
        Err(errors) => panic!("{name}:\n{}", report(name, &errors)),
    }
}

// W5 — arrays and indexing.

#[test]
fn a_lua_list_becomes_a_fixed_length_array() {
    // What a palette or a convolution kernel wants to be, and what a Lua author
    // will write without being told.
    let out = emitted(
        "palette array",
        "function fragment(uv)
           local steps = { 0.0, 0.35, 0.7, 1.0 }
           local i = i32(uv.x * 4.0)
           return vec4(steps[i], 0.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("array<f32, 4>("), "{out}");
    assert!(out.contains("["), "and it is indexed: {out}");
}

#[test]
fn an_array_of_vectors_works_too() {
    let out = emitted(
        "colour ramp",
        "function fragment(uv)
           local ramp = { vec3(1.0, 0.2, 0.1), vec3(0.1, 0.8, 0.3), vec3(0.2, 0.3, 1.0) }
           local i = i32(uv.x * 3.0)
           return vec4(ramp[i], 1.0)
         end",
    );
    assert!(out.contains("array<vec3<f32>, 3>("), "{out}");
}

#[test]
fn a_vector_and_a_matrix_can_be_indexed() {
    // The note that used to point at `v.x` was right until arrays existed and
    // then became a wrong answer.
    let out = emitted(
        "indexing",
        "function fragment(uv)
           local m = mat3(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0))
           local column = m[1]
           local component = column[2]
           local from_vector = uv[0]
           return vec4(component, from_vector, 0.0, 1.0)
         end",
    );
    assert!(out.contains("[1]"), "{out}");
    assert!(out.contains("[2]"), "{out}");
}

#[test]
fn a_mixed_list_says_every_element_is_one_type() {
    let found = errors(
        "function fragment(uv)
           local mixed = { 1.0, vec2(0.0, 1.0) }
           return vec4(mixed[0], 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "mixes f32 and vec2"), "{found:?}");
    assert!(mentions(&found, "same type"), "{found:?}");
}

#[test]
fn indexing_with_a_float_says_to_convert() {
    // WGSL indexes with a whole number, and rounding one silently is how a
    // shader reads the wrong element and nobody finds out.
    let found = errors(
        "function fragment(uv)
           local steps = { 0.0, 1.0 }
           return vec4(steps[uv.x], 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "whole number"), "{found:?}");
    assert!(mentions(&found, "i32(n)"), "{found:?}");
}

#[test]
fn indexing_a_number_says_what_can_be() {
    let found = errors(
        "function fragment(uv)
           local x = 1.0
           return vec4(x[0], 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "cannot be indexed"), "{found:?}");
    assert!(
        mentions(&found, "arrays, vectors and matrices"),
        "{found:?}"
    );
}

#[test]
fn an_array_parameter_is_packed_at_the_uniform_stride() {
    // In the uniform address space an array's stride is a multiple of sixteen
    // whatever the element is — the rule that rejected the first attempt at
    // padding the parameter block, back in M4.
    let params = vec![Binding {
        name: "ramp".to_owned(),
        ty: Type::array(Type::F32, 4),
    }];
    let compiled = compile_with(
        "function fragment(uv, time, resolution, coverage, ramp)
           return vec4(ramp[0], ramp[1], ramp[2], ramp[3])
         end",
        ShaderKind::Material,
        params,
    )
    .expect("this compiles");
    assert_eq!(compiled.params[0].offset, HEADER_BYTES);
    // Four f32 at a sixteen-byte stride is sixty-four bytes, not sixteen.
    assert_eq!(compiled.uniform_size, HEADER_BYTES + 64);
}
