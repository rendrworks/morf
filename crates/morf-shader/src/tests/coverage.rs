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
