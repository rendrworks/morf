use super::*;

// What the compiler emits, asserted on fragments rather than whole modules so
// that adding a uniform does not break every test in the file.

#[test]
fn arithmetic_lowers_to_wgsl_operators() {
    let out = wgsl(
        "function fragment(uv)
           local d = uv.x * 2.0 + 1.0
           return vec4(d, d, d, 1.0)
         end",
    );
    assert!(out.contains("((morf_in0.x * 2.0) + 1.0)"), "{out}");
    assert!(out.contains("-> vec4<f32>"), "{out}");
}

#[test]
fn precedence_comes_from_the_parser_not_from_us() {
    // `a + b * c` must be `a + (b * c)`. The parser nests the higher-precedence
    // operand, so a left fold is correct — this is the test that says so.
    let out = wgsl(
        "function fragment(uv)
           local d = uv.x + uv.y * 4.0
           return vec4(d)
         end",
    );
    assert!(out.contains("(morf_in0.x + (morf_in0.y * 4.0))"), "{out}");
}

#[test]
fn an_integer_literal_is_a_float_so_division_matches_lua() {
    // `1 / 2` is 0.5 in Lua. Emitting integer literals would make it 0 in WGSL,
    // silently, which is the kind of difference nobody finds by reading.
    let out = wgsl(
        "function fragment(uv)
           local half = 1 / 2
           return vec4(half)
         end",
    );
    assert!(out.contains("(1.0 / 2.0)"), "{out}");
}

#[test]
fn a_scalar_widens_to_the_vector_it_is_combined_with() {
    let out = wgsl(
        "function fragment(uv)
           local c = vec3(1.0, 0.0, 0.0) * 0.5
           return vec4(c, 1.0)
         end",
    );
    assert!(out.contains("vec3<f32>(0.5)"), "{out}");
}

#[test]
fn swizzles_select_components() {
    let out = wgsl(
        "function fragment(uv)
           local c = vec4(1.0, 0.5, 0.25, 1.0)
           return vec4(c.rgb, c.a)
         end",
    );
    assert!(out.contains(".xyz"), "{out}");
    assert!(out.contains(".w"), "{out}");
}

#[test]
fn builtins_lower_by_name_under_either_spelling() {
    for source in ["sin(uv.x)", "math.sin(uv.x)"] {
        let out = wgsl(&format!(
            "function fragment(uv)
               return vec4(vec3({source}), 1.0)
             end"
        ));
        assert!(out.contains("sin(morf_in0.x)"), "{source}: {out}");
    }
}

#[test]
fn a_branch_becomes_a_wgsl_if() {
    let out = wgsl(
        "function fragment(uv)
           if uv.x > 0.5 then
             return vec4(1.0, 0.0, 0.0, 1.0)
           else
             return vec4(0.0, 0.0, 1.0, 1.0)
           end
         end",
    );
    assert!(out.contains("if ((morf_in0.x > 0.5))"), "{out}");
    assert!(out.contains("} else {"), "{out}");
}

#[test]
fn every_loop_carries_a_guard() {
    // The single most important property in the crate: a configuration cannot
    // write a loop that runs away, because it does not decide what the loop is.
    let out = wgsl(
        "function fragment(uv)
           local total = 0.0
           local i = 0.0
           while i < 10.0 do
             total = total + 0.1
             i = i + 1.0
           end
           return vec4(total)
         end",
    );
    assert!(out.contains("var morf_guard0: u32 = 0u;"), "{out}");
    assert!(
        out.contains(&format!("if (morf_guard0 >= {MAX_ITERATIONS}u)")),
        "{out}"
    );
    assert!(out.contains("morf_guard0 = morf_guard0 + 1u;"), "{out}");
}

#[test]
fn an_endless_loop_still_terminates() {
    // `while true do end` is the shape that kills a compositor. It compiles,
    // and what comes out has a bound on it.
    let out = wgsl(
        "function fragment(uv)
           while true do
           end
           return vec4(1.0)
         end",
    );
    assert!(out.contains("morf_guard0 >= "), "{out}");
}

#[test]
fn a_numeric_for_counts_and_is_bounded() {
    let out = wgsl(
        "function fragment(uv)
           local total = 0.0
           for i = 1, 8 do
             total = total + 0.1
           end
           return vec4(total)
         end",
    );
    assert!(out.contains("morf_guard0"), "{out}");
    assert!(out.contains("+ 1.0"), "{out}");
}

#[test]
fn a_descending_for_compares_the_other_way() {
    let out = wgsl(
        "function fragment(uv)
           local total = 0.0
           for i = 8, 1, -1 do
             total = total + 0.1
           end
           return vec4(total)
         end",
    );
    // Counting down ends when the counter falls below the limit, not above it.
    assert!(out.contains(" < "), "{out}");
}

#[test]
fn a_local_that_is_never_written_is_emitted_as_let() {
    let out = wgsl(
        "function fragment(uv)
           local d = uv.x
           return vec4(d)
         end",
    );
    assert!(out.contains("let d_"), "{out}");
    assert!(!out.contains("var d_"), "{out}");
}

#[test]
fn a_local_that_is_written_is_emitted_as_var() {
    let out = wgsl(
        "function fragment(uv)
           local d = uv.x
           d = d + 1.0
           return vec4(d)
         end",
    );
    assert!(out.contains("var d_"), "{out}");
}

#[test]
fn params_are_packed_with_wgsl_alignment() {
    let params = vec![
        Binding {
            name: "intensity".to_owned(),
            ty: Type::F32,
        },
        Binding {
            name: "tint".to_owned(),
            ty: Type::Vec4,
        },
    ];
    let compiled = compile_with(
        "function fragment(uv, time, resolution, coverage, intensity, tint)
           return tint * intensity
         end",
        ShaderKind::Material,
        params,
    )
    .expect("this compiles");
    assert_eq!(compiled.params[0].offset, 0);
    // A vec4 aligns to sixteen, so the f32 before it cannot simply be followed.
    assert_eq!(compiled.params[1].offset, 16);
    assert_eq!(compiled.uniform_size, 32);
}

#[test]
fn reading_the_clock_is_recorded_and_not_reading_it_is_too() {
    let still = compile_material(
        "function fragment(uv)
           return vec4(uv.x)
         end",
    )
    .expect("compiles");
    assert!(!still.reads_time, "a shader without time does not animate");

    let moving = compile_material(
        "function fragment(uv, time)
           return vec4(sin(time))
         end",
    )
    .expect("compiles");
    assert!(moving.reads_time, "a shader reading time repaints");
}

#[test]
fn identical_shaders_share_a_hash_and_different_ones_do_not() {
    let one = compile_material(
        "function fragment(uv)
           return vec4(uv.x)
         end",
    )
    .expect("compiles");
    let same = compile_material(
        "function fragment(uv)
           -- a comment changes the source but not the shader
           return vec4(uv.x)
         end",
    )
    .expect("compiles");
    let other = compile_material(
        "function fragment(uv)
           return vec4(uv.y)
         end",
    )
    .expect("compiles");
    assert_eq!(one.hash, same.hash, "comments do not make a new pipeline");
    assert_ne!(one.hash, other.hash);
}
