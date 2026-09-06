use super::*;

// The remainder of PLAN_WGSL_COVERAGE.md: the packing family, the coarse and
// fine derivatives, `ldexp`, `outer`, and `modf`/`frexp`.
//
// These were each deferred once with a reason. The reasons were real but they
// were not the same as being done, and a plan with unchecked boxes in it is not
// an implemented plan.

fn emitted(name: &str, body: &str) -> String {
    match compile_material(body) {
        Ok(compiled) => compiled.wgsl,
        Err(errors) => panic!("{name}:\n{}", report(name, &errors)),
    }
}

#[test]
fn ldexp_scales_by_a_power_of_two() {
    // The straight miss: W1 deferred this to W3 for needing an `i32`
    // exponent, and W3 added integers without coming back for it.
    let out = emitted(
        "ldexp",
        "function fragment(uv)
           local scaled = ldexp(uv.x, i32(3))
           return vec4(scaled, 0.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("ldexp("), "{out}");
}

#[test]
fn outer_products_build_a_matrix() {
    // WGSL has no `outer`. naga carries one for its GLSL frontend and the WGSL
    // grammar does not name it, so calling it emits WGSL no driver will accept
    // — which is what the GPU test found. It is a matrix of scaled copies of
    // one vector, so it is emitted as exactly that.
    let out = emitted(
        "outer",
        "function fragment(uv)
           local m = outer(vec3(1.0, 2.0, 3.0), vec3(0.5, 0.5, 0.5))
           return vec4(m * vec3(uv.x, uv.y, 1.0), 1.0)
         end",
    );
    assert!(!out.contains("outer("), "not as a call: {out}");
    assert!(out.contains("mat3x3<f32>("), "as a matrix: {out}");
    // Three columns, each the first vector scaled by a component of the second.
    for component in [".x", ".y", ".z"] {
        assert!(
            out.contains(component),
            "column {component} missing:\n{out}"
        );
    }
}

#[test]
fn an_outer_product_that_is_not_square_says_so() {
    // This language has only square matrices, and naming the limit is better
    // than emitting a type it cannot spell.
    let found = errors(
        "function fragment(uv)
           local m = outer(vec3(1.0, 2.0, 3.0), vec2(0.5, 0.5))
           return vec4(1.0)
         end",
    );
    assert!(mentions(&found, "not square"), "{found:?}");
}

#[test]
fn the_coarse_and_fine_derivatives_are_available() {
    // Deferred in W4 as "a precision hint nobody writes". They are still that,
    // and they are also six table entries, which is less than the argument for
    // leaving them out was worth.
    let out = emitted(
        "derivative variants",
        "function fragment(uv)
           local a = dpdx_coarse(uv.x) + dpdx_fine(uv.x)
           local b = dpdy_coarse(uv.y) + dpdy_fine(uv.y)
           local c = fwidth_coarse(uv.x) + fwidth_fine(uv.y)
           return vec4(a * 10.0, b * 10.0, c * 10.0, 1.0)
         end",
    );
    for call in [
        "dpdxCoarse(",
        "dpdxFine(",
        "dpdyCoarse(",
        "dpdyFine(",
        "fwidthCoarse(",
        "fwidthFine(",
    ] {
        assert!(out.contains(call), "{call} missing:\n{out}");
    }
}

#[test]
fn a_coarse_derivative_obeys_the_same_uniformity_rule() {
    // Six more calls under one rule, and the rule already knew how to name
    // whichever it found.
    let found = errors(
        "function fragment(uv)
           local d = 0.0
           if uv.x > 0.5 then
             d = fwidth_fine(uv.y)
           end
           return vec4(d, 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "fwidthFine"), "{found:?}");
}

#[test]
fn the_float_packing_family_is_present() {
    let out = emitted(
        "packing",
        "function fragment(uv)
           local four = vec4(uv.x, uv.y, 0.5, 1.0)
           local two = vec2(uv.x, uv.y)
           local bits = pack4x8unorm(four)
             + pack4x8snorm(four)
             + pack2x16unorm(two)
             + pack2x16snorm(two)
             + pack2x16float(two)
           local back = unpack4x8unorm(bits) + unpack4x8snorm(bits)
           local pair = unpack2x16float(bits) + unpack2x16unorm(bits)
           return vec4(back.xy + pair, 0.0, 1.0)
         end",
    );
    for call in [
        "pack4x8unorm(",
        "pack4x8snorm(",
        "pack2x16unorm(",
        "pack2x16snorm(",
        "pack2x16float(",
        "unpack4x8unorm(",
        "unpack4x8snorm(",
        "unpack2x16float(",
        "unpack2x16unorm(",
    ] {
        assert!(out.contains(call), "{call} missing:\n{out}");
    }
}

#[test]
fn the_byte_packing_family_needs_integer_vectors_and_has_them() {
    // Six of the eighteen packing builtins take or return a `vec4<i32>` or a
    // `vec4<u32>`, which is why they could not simply be table entries: the
    // types had to exist first.
    let out = emitted(
        "byte packing",
        "function fragment(uv)
           local signed = vec4i(1, -2, 3, -4)
           local unsigned = vec4u(1, 2, 3, 4)
           local bits = pack4x_i8(signed)
             + pack4x_u8(unsigned)
             + pack4x_i8_clamp(signed)
             + pack4x_u8_clamp(unsigned)
           local back = unpack4x_u8(bits)
           local dotted = dot4_u8_packed(bits, bits)
           return vec4(f32(back.x + dotted) * 0.001, 0.0, 0.0, 1.0)
         end",
    );
    for call in [
        "pack4xI8(",
        "pack4xU8(",
        "pack4xI8Clamp(",
        "pack4xU8Clamp(",
        "unpack4xU8(",
        "dot4U8Packed(",
    ] {
        assert!(out.contains(call), "{call} missing:\n{out}");
    }
    assert!(out.contains("vec4<i32>("), "integer vectors exist: {out}");
}

#[test]
fn modf_and_frexp_split_a_number() {
    // WGSL returns a struct from both, and this language has no struct of its
    // own. Rather than inventing one, the result is a type whose only operation
    // is reading a part off it — which is the whole of what anybody does.
    let out = emitted(
        "split",
        "function fragment(uv)
           local parts = modf(uv.x * 4.0)
           local pieces = frexp(uv.y + 1.0)
           return vec4(parts.fract, parts.whole * 0.1, f32(pieces.exp) * 0.1, 1.0)
         end",
    );
    assert!(out.contains("modf("), "{out}");
    assert!(out.contains(".fract"), "{out}");
    assert!(out.contains("frexp("), "{out}");
    assert!(out.contains(".exp"), "{out}");
}

#[test]
fn a_split_result_names_the_parts_it_has() {
    let found = errors(
        "function fragment(uv)
           local parts = modf(uv.x)
           return vec4(parts.nonsense, 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "no `nonsense`"), "{found:?}");
    assert!(mentions(&found, "`.fract` and `.whole`"), "{found:?}");
}

#[test]
fn every_naga_math_function_this_language_can_reach_is_reachable() {
    // The count the plan tracks, checked rather than remembered. naga 30.0.1's
    // `MathFunction` has 79 variants; `Outer` is the only one this language
    // deliberately restricts (to the square case), and every other one is
    // callable by name.
    let names = crate::builtins::available();
    let count = names.split(", ").count();
    assert!(
        count >= 79,
        "expected the whole surface, found {count}: {names}",
    );
}

#[test]
fn a_lua_record_becomes_a_struct() {
    // The last unchecked compiler-side box. There is nowhere in Lua to declare
    // a struct, so identity comes from the shape: two tables with the same
    // field names and types are the same type.
    let out = emitted(
        "record",
        "function fragment(uv)
           local light = { colour = vec3(1.0, 0.9, 0.8), power = 2.5 }
           return vec4(light.colour * light.power * uv.x, 1.0)
         end",
    );
    assert!(out.contains("struct MorfRecord"), "{out}");
    assert!(out.contains("colour: vec3<f32>,"), "{out}");
    assert!(out.contains("power: f32,"), "{out}");
    assert!(out.contains(".colour"), "and its fields are read: {out}");
}

#[test]
fn two_records_of_the_same_shape_are_one_type() {
    // Structural typing, and the reason the fields are sorted: a Lua table has
    // no order of its own, so two records differing only in the order they were
    // written are the same record.
    let out = emitted(
        "same shape",
        "function fragment(uv)
           local a = { x = 1.0, y = 2.0 }
           local b = { y = 3.0, x = 4.0 }
           return vec4(a.x + b.x, a.y + b.y, 0.0, 1.0)
         end",
    );
    let declarations = out.matches("struct MorfRecord").count();
    assert_eq!(declarations, 1, "one declaration, not two:\n{out}");
}

#[test]
fn a_record_field_that_does_not_exist_is_named() {
    let found = errors(
        "function fragment(uv)
           local light = { power = 2.0 }
           return vec4(light.colour, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "has no `colour`"), "{found:?}");
    assert!(
        mentions(&found, "only the fields it was written with"),
        "{found:?}"
    );
}
