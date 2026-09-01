use super::*;

// Coverage against WGSL, tracked in PLAN_WGSL_COVERAGE.md.
//
// One test per group of builtins, plus the diagnostics for using them wrongly.
// The point of testing a builtin at all is that a wrong overload should say
// what was wrong rather than emit WGSL a driver will reject with no line
// number.

/// The WGSL for a body expected to compile, with the shader's own name in the
/// panic when it does not.
fn emitted(name: &str, body: &str) -> String {
    match compile_material(body) {
        Ok(compiled) => compiled.wgsl,
        Err(errors) => panic!("{name}:\n{}", report(name, &errors)),
    }
}

#[test]
fn the_ordinary_arithmetic_builtins_are_present() {
    // W1. Each is one table entry, and each is here because a shader author
    // reaching for it and not finding it is the failure being prevented.
    for (call, expected) in [
        ("saturate(uv.x)", "saturate("),
        ("trunc(uv.x)", "trunc("),
        ("inversesqrt(uv.x + 1.0)", "inverseSqrt("),
        ("inverse_sqrt(uv.x + 1.0)", "inverseSqrt("),
        ("fma(uv.x, 2.0, 1.0)", "fma("),
        ("asinh(uv.x)", "asinh("),
        ("acosh(uv.x + 1.0)", "acosh("),
        ("atanh(uv.x * 0.5)", "atanh("),
        ("quantize_to_f16(uv.x)", "quantizeToF16("),
    ] {
        let out = emitted(
            call,
            &format!(
                "function fragment(uv)
                   return vec4({call}, 0.0, 0.0, 1.0)
                 end"
            ),
        );
        assert!(out.contains(expected), "{call} emitted:\n{out}");
    }
}

#[test]
fn the_vector_geometry_builtins_are_present() {
    let out = emitted(
        "geometry",
        "function fragment(uv)
           local i = vec3(0.3, -0.8, 0.5)
           local n = vec3(0.0, 1.0, 0.0)
           local bounced = reflect(i, n)
           local bent = refract(i, n, 0.66)
           local facing = faceforward(n, i, n)
           return vec4(bounced + bent + facing, 1.0)
         end",
    );
    assert!(out.contains("refract("), "{out}");
    assert!(out.contains("faceForward("), "{out}");
}

#[test]
fn refract_says_what_its_third_argument_has_to_be() {
    // The one W1 builtin with a shape of its own: two vectors and a scalar
    // ratio, not three of one type.
    let found = errors(
        "function fragment(uv)
           local i = vec3(0.3, -0.8, 0.5)
           return vec4(refract(i, i, vec3(0.5, 0.5, 0.5)), 1.0)
         end",
    );
    assert!(mentions(&found, "ratio must be an f32"), "{found:?}");
}

#[test]
fn a_builtin_used_at_the_wrong_type_names_both() {
    let found = errors(
        "function fragment(uv)
           return vec4(saturate(true), 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "saturate"), "{found:?}");
}

/// Every builtin name the language offers, so the count in the plan is checked
/// rather than remembered.
#[test]
fn the_builtin_count_matches_what_the_plan_claims() {
    // naga 30.0.1's `MathFunction` has 79 variants. This is not a target to
    // reach — packing and unpacking exist for storage buffers a shader here
    // does not have — but the number should never drift without somebody
    // noticing, because "we have most of it" is how a gap goes unmeasured.
    let names = crate::builtins::available();
    let count = names.split(", ").count();
    assert!(
        count >= 46,
        "W1 brings the surface to at least 46 names, found {count}: {names}",
    );
}

// W2 — matrices.

#[test]
fn a_rotation_matrix_can_be_written_and_applied() {
    // The thing W2 exists for. Before it, this could only be written by
    // expanding the arithmetic by hand, which is why every Shadertoy port that
    // turns anything wanted it.
    let out = emitted(
        "rotate",
        "function fragment(uv, time)
           local c = cos(time)
           local s = sin(time)
           local turn = mat2(vec2(c, s), vec2(0.0 - s, c))
           local p = turn * (uv - vec2(0.5, 0.5))
           return vec4(p + vec2(0.5, 0.5), 0.0, 1.0)
         end",
    );
    assert!(out.contains("mat2x2<f32>("), "{out}");
    assert!(out.contains(" * "), "and it is applied");
}

#[test]
fn a_matrix_can_be_built_from_columns_or_from_numbers() {
    // WGSL accepts both, and so does this: columns are how a rotation is
    // usually written, and the flat form is how one gets pasted out of
    // somebody else's shader.
    for source in [
        "mat3(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0))",
        "mat3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0)",
    ] {
        let out = emitted(
            source,
            &format!(
                "function fragment(uv)
                   local m = {source}
                   local v = m * vec3(uv.x, uv.y, 1.0)
                   return vec4(v, 1.0)
                 end"
            ),
        );
        assert!(out.contains("mat3x3<f32>("), "{source}: {out}");
    }
}

#[test]
fn a_matrix_multiplies_vectors_matrices_and_scalars() {
    let out = emitted(
        "products",
        "function fragment(uv)
           local m = mat2(vec2(2.0, 0.0), vec2(0.0, 2.0))
           local scaled = m * 0.5
           local composed = m * scaled
           local applied = composed * uv
           local row = uv * composed
           return vec4(applied + row, 0.0, 1.0)
         end",
    );
    // A scalar never widens into a matrix: `mat2x2<f32>(0.5)` is not legal.
    assert!(!out.contains("mat2x2<f32>(0.5)"), "{out}");
}

#[test]
fn the_matrix_builtins_are_present() {
    let out = emitted(
        "matrix builtins",
        "function fragment(uv)
           local m = mat3(vec3(2.0, 0.0, 0.0), vec3(0.0, 2.0, 0.0), vec3(0.0, 0.0, 1.0))
           local d = determinant(m)
           local back = inverse(transpose(m))
           local v = back * vec3(uv.x, uv.y, 1.0)
           return vec4(v * d, 1.0)
         end",
    );
    for call in ["determinant(", "inverse(", "transpose("] {
        assert!(out.contains(call), "{call} missing:\n{out}");
    }
}

#[test]
fn adding_a_matrix_to_something_is_refused() {
    // `m * v` is a linear map applied to a vector, not a componentwise
    // multiply, and `m + v` has no meaning worth guessing at.
    let found = errors(
        "function fragment(uv)
           local m = mat2(vec2(1.0, 0.0), vec2(0.0, 1.0))
           return vec4(m + uv, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "mat2"), "{found:?}");
}

#[test]
fn a_matrix_of_the_wrong_shape_says_what_it_wanted() {
    let found = errors(
        "function fragment(uv)
           local m = mat3(vec2(1.0, 0.0), vec2(0.0, 1.0))
           return vec4(m * vec3(1.0, 1.0, 1.0), 1.0)
         end",
    );
    assert!(mentions(&found, "cannot be built from"), "{found:?}");
    assert!(mentions(&found, "3 vec3 columns"), "{found:?}");
}

#[test]
fn a_mismatched_matrix_product_is_refused() {
    let found = errors(
        "function fragment(uv)
           local m = mat3(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0))
           return vec4(m * uv, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "mat3"), "{found:?}");
    assert!(mentions(&found, "vec2"), "{found:?}");
}

#[test]
fn matrix_parameters_are_packed_at_wgsl_alignment() {
    // A `mat3x3` is three sixteen-byte columns — forty-eight bytes, not
    // thirty-six. The rule that surprises everybody, and the reason the packer
    // computes rather than assumes.
    let params = vec![
        Binding {
            name: "level".to_owned(),
            ty: Type::F32,
        },
        Binding {
            name: "turn".to_owned(),
            ty: Type::Mat3,
        },
    ];
    let compiled = compile_with(
        "function fragment(uv, time, resolution, coverage, level, turn)
           return vec4(turn * vec3(uv.x, uv.y, level), 1.0)
         end",
        ShaderKind::Material,
        params,
    )
    .expect("this compiles");
    assert_eq!(compiled.params[0].offset, HEADER_BYTES);
    // The matrix aligns to sixteen, so it cannot simply follow the f32.
    assert_eq!(compiled.params[1].offset, HEADER_BYTES + 16);
    assert_eq!(compiled.uniform_size, HEADER_BYTES + 16 + 48);
}

// W3 — integers and bitwise.

#[test]
fn an_integer_literal_still_divides_like_lua() {
    // The rule abstract integers exist to preserve. Nothing in `1 / 2` asks for
    // a whole number, so it stays a float and the answer is a half — as Lua
    // gives, and unlike C.
    let out = emitted(
        "division",
        "function fragment(uv)
           local half = 1 / 2
           return vec4(half, half, half, 1.0)
         end",
    );
    assert!(out.contains("(1.0 / 2.0)"), "{out}");
}

#[test]
fn an_integer_literal_stays_whole_where_something_asks_it_to() {
    // The other half of the same rule: a shift asks for whole numbers, so the
    // literals become them rather than being refused.
    let out = emitted(
        "shift",
        "function fragment(uv)
           local four = 1 << 2
           return vec4(f32(four) * 0.1, 0.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("(1 << 2)"), "{out}");
    assert!(!out.contains("(1.0 << 2.0)"), "and not as floats: {out}");
}

#[test]
fn a_hash_constant_survives_that_would_not_fit_a_float() {
    // Why this step exists at all. `2654435769` needs thirty-two bits and an
    // `f32` has twenty-four of mantissa, so before abstract integers there was
    // no way to write a hash — and therefore no way to write noise that was not
    // the `sin(dot(p, k)) * 43758.5453` trick.
    let out = emitted(
        "hash",
        "function fragment(uv)
           local h = bitcast_u32(uv.x)
           h = h * u32(2654435769)
           h = h ~ (h >> u32(15))
           return vec4(f32(h & u32(255)) / 255.0, 0.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("2654435769u"), "the constant survived: {out}");
    assert!(out.contains("bitcast<u32>("), "{out}");
}

#[test]
fn the_bitwise_operators_are_all_present() {
    let out = emitted(
        "bitwise",
        "function fragment(uv)
           local a = u32(12)
           local b = u32(10)
           local mixed = ((a & b) | (a ~ b)) << u32(1)
           local back = (~mixed) >> u32(2)
           return vec4(f32(back & u32(255)) / 255.0, 0.0, 0.0, 1.0)
         end",
    );
    for operator in ["&", "|", "^", "<<", ">>", "~("] {
        assert!(out.contains(operator), "{operator} missing:\n{out}");
    }
}

#[test]
fn the_bit_counting_builtins_are_present() {
    let out = emitted(
        "bit builtins",
        "function fragment(uv)
           local h = u32(305419896)
           local counted = count_one_bits(h)
             + count_leading_zeros(h)
             + count_trailing_zeros(h)
             + reverse_bits(h)
             + first_leading_bit(h)
             + first_trailing_bit(h)
             + extract_bits(h, u32(4), u32(8))
             + insert_bits(h, u32(3), u32(1), u32(2))
           return vec4(f32(counted & u32(255)) / 255.0, 0.0, 0.0, 1.0)
         end",
    );
    for call in [
        "countOneBits(",
        "countLeadingZeros(",
        "countTrailingZeros(",
        "reverseBits(",
        "firstLeadingBit(",
        "firstTrailingBit(",
        "extractBits(",
        "insertBits(",
    ] {
        assert!(out.contains(call), "{call} missing:\n{out}");
    }
}

#[test]
fn shifting_a_float_says_to_convert_first() {
    let found = errors(
        "function fragment(uv)
           local shifted = uv.x << 2
           return vec4(shifted, 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "works on whole numbers"), "{found:?}");
    assert!(mentions(&found, "u32(x)"), "{found:?}");
}

#[test]
fn conversions_go_both_ways() {
    let out = emitted(
        "conversions",
        "function fragment(uv)
           local whole = i32(uv.x * 10.0)
           local back = f32(whole) * 0.1
           local unsigned = u32(whole)
           return vec4(back, f32(unsigned) * 0.001, 0.0, 1.0)
         end",
    );
    assert!(out.contains("i32("), "{out}");
    assert!(out.contains("f32("), "{out}");
    assert!(out.contains("u32("), "{out}");
}

#[test]
fn a_bool_has_no_bits_to_reinterpret() {
    let found = errors(
        "function fragment(uv)
           return vec4(f32(bitcast_u32(true)), 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "no bits to reinterpret"), "{found:?}");
}

// W4 — derivatives and relational.

#[test]
fn a_shader_can_antialias_its_own_edge() {
    // The asymmetry W4 closes. `field.wgsl` softens its own edges with
    // `fwidth`, and before this a configuration's shader could not — so a
    // shader that drew its own shape had no way to soften it at the resolution
    // it was actually being drawn at.
    let out = emitted(
        "antialias",
        "function fragment(uv)
           local d = length(uv - vec2(0.5, 0.5)) - 0.3
           local edge = fwidth(d)
           local coverage = 1.0 - smoothstep(0.0 - edge, edge, d)
           return vec4(coverage, coverage, coverage, 1.0)
         end",
    );
    assert!(out.contains("fwidth("), "{out}");
}

#[test]
fn both_derivative_axes_are_available() {
    let out = emitted(
        "derivatives",
        "function fragment(uv)
           local gx = dpdx(uv.x)
           local gy = dpdy(uv.y)
           return vec4(abs(gx) * 50.0, abs(gy) * 50.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("dpdx("), "{out}");
    assert!(out.contains("dpdy("), "{out}");
}

#[test]
fn the_relational_builtins_are_present() {
    let out = emitted(
        "relational",
        "function fragment(uv)
           local bad = is_nan(uv.x) or is_inf(uv.y)
           local lit = select(1.0, 0.0, all(bad))
           return vec4(lit, select(0.0, 1.0, any(bad)), 0.0, 1.0)
         end",
    );
    for call in ["isNan(", "isInf(", "all(", "any("] {
        assert!(out.contains(call), "{call} missing:\n{out}");
    }
}

#[test]
fn a_derivative_inside_a_branch_is_refused_by_name() {
    // WGSL's uniformity rule, caught here rather than by naga — whose message
    // for it is close to unreadable and carries no line number. One check
    // covers `texture` and all three derivatives, because it is one rule.
    let found = errors(
        "function fragment(uv)
           local d = 0.0
           if uv.x > 0.5 then
             d = fwidth(uv.y)
           end
           return vec4(d, 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "fwidth"), "{found:?}");
    assert!(mentions(&found, "inside an `if`"), "{found:?}");
    assert!(mentions(&found, "neighbouring pixels"), "{found:?}");
}

#[test]
fn a_derivative_inside_a_loop_is_refused_too() {
    let found = errors(
        "function fragment(uv)
           local total = 0.0
           local i = 0.0
           while i < 4.0 do
             i = i + 1.0
             total = total + dpdx(uv.x)
           end
           return vec4(total, 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "dpdx"), "{found:?}");
}

#[test]
fn a_derivative_outside_control_flow_is_fine() {
    // The rule is about *where*, not about the call. Computing it first and
    // branching on the result is exactly what the diagnostic suggests, so it
    // had better work.
    let out = emitted(
        "hoisted",
        "function fragment(uv)
           local edge = fwidth(uv.x)
           local shade = 0.0
           if uv.x > 0.5 then
             shade = edge * 40.0
           end
           return vec4(shade, 0.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("fwidth("), "{out}");
}
