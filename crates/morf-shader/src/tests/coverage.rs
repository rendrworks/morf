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

// W6 — `continue` and `discard`.

#[test]
fn continue_is_spelled_the_way_lua_spells_it() {
    // Lua has no `continue`, and the idiom every Lua author already writes is
    // `goto continue` with a `::continue::` label at the end of the body. That
    // is real Lua syntax rather than something invented here, so it is what a
    // shader uses — no new keyword, and nothing to learn.
    let out = emitted(
        "continue",
        "function fragment(uv)
           local total = 0.0
           for i = 1, 8 do
             if i > 4.0 then
               goto continue
             end
             total = total + 0.1
             ::continue::
           end
           return vec4(total, 0.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("continue;"), "{out}");
}

#[test]
fn a_counting_loop_still_advances_after_a_continue() {
    // The bug this had to avoid. A `continue` jumps past the tail of the body,
    // so an increment left there would turn a counting loop into one that never
    // advances — bounded by the guard, but wrong. WGSL's `continuing` block
    // exists for exactly this, and the counter lives in it.
    let out = emitted(
        "counting continue",
        "function fragment(uv)
           local total = 0.0
           for i = 1, 4 do
             goto continue
             ::continue::
           end
           return vec4(total, 0.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("continuing {"), "{out}");
    // The increment is inside it, not before the closing brace of the body.
    let continuing = out.split("continuing {").nth(1).unwrap_or_default();
    assert!(
        continuing.contains("+ 1.0"),
        "the counter advances there: {out}"
    );
}

#[test]
fn a_while_loop_needs_no_continuing_block() {
    // Its condition is already re-checked at the top of the body, so there is
    // nothing that has to happen on the way round. An empty `continuing` would
    // be noise in every generated shader.
    let out = emitted(
        "while continue",
        "function fragment(uv)
           local i = 0.0
           while i < 4.0 do
             i = i + 1.0
           end
           return vec4(i * 0.1, 0.0, 0.0, 1.0)
         end",
    );
    assert!(!out.contains("continuing {"), "{out}");
}

#[test]
fn continue_outside_a_loop_is_refused() {
    let found = errors(
        "function fragment(uv)
           goto continue
         end",
    );
    assert!(mentions(&found, "outside a loop"), "{found:?}");
}

#[test]
fn any_other_goto_still_says_what_is_available() {
    let found = errors(
        "function fragment(uv)
           goto elsewhere
           ::elsewhere::
           return vec4(1.0)
         end",
    );
    assert!(mentions(&found, "not available"), "{found:?}");
    assert!(mentions(&found, "goto continue"), "{found:?}");
}

#[test]
fn discard_throws_the_fragment_away() {
    // Spelled as a call because Lua has no keyword to spare, and it is the one
    // call whose entire point is its effect rather than its value.
    let out = emitted(
        "discard",
        "function fragment(uv)
           if uv.x > 0.9 then
             discard()
           end
           return vec4(1.0, 0.0, 0.0, 1.0)
         end",
    );
    assert!(out.contains("discard;"), "{out}");
}
