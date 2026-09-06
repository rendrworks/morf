use super::*;

// The errors a configuration author will actually hit, and the exact wording
// each has to produce. Ranked by how often they will happen.
//
// This file is the reason error quality does not rot: a message can be improved
// freely, but it cannot quietly stop being helpful.

#[test]
fn a_number_as_a_condition_is_refused_rather_than_coerced() {
    // The single most important diagnostic here. Lua treats every value but
    // `nil` and `false` as true, so an author writes `if x then` meaning
    // `if x > 0.0 then`. Coercing would emit a shader that is wrong with no
    // error at all, which is worse than any error message.
    let found = errors(
        "function fragment(uv)
           if uv.x then
             return vec4(1.0)
           end
           return vec4(0.0)
         end",
    );
    assert!(mentions(&found, "must be a bool"), "{found:?}");
    assert!(mentions(&found, "x > 0.0"), "{found:?}");
    assert_eq!(found[0].line, 2, "points at the `if`");
}

#[test]
fn mismatched_vector_arithmetic_names_both_types() {
    let found = errors(
        "function fragment(uv)
           local bad = vec3(1.0) + vec2(1.0)
           return vec4(bad, 1.0)
         end",
    );
    assert!(mentions(&found, "vec3"), "{found:?}");
    assert!(mentions(&found, "vec2"), "{found:?}");
}

#[test]
fn an_empty_table_has_no_type() {
    // A list is an array now, but an empty one cannot be: a shader array's
    // length and element type are part of what it *is*, and there is nothing
    // in `{}` to read either from.
    let found = errors(
        "function fragment(uv)
           local t = {}
           return vec4(1.0)
         end",
    );
    assert!(mentions(&found, "no type"), "{found:?}");
    assert!(mentions(&found, "part of what it is"), "{found:?}");
}

#[test]
fn a_table_cannot_be_a_list_and_a_record_at_once() {
    // Named fields are a record now, and a list is an array. What has no shape
    // is a table trying to be both.
    let found = errors(
        "function fragment(uv)
           local t = { 1.0, red = 0.5 }
           return vec4(1.0)
         end",
    );
    assert!(mentions(&found, "mixes"), "{found:?}");
    assert!(mentions(&found, "one or the other"), "{found:?}");
}

#[test]
fn an_unknown_function_lists_what_is_available() {
    let found = errors(
        "function fragment(uv)
           return vec4(wobble(uv.x))
         end",
    );
    assert!(mentions(&found, "not a shader function"), "{found:?}");
    assert!(mentions(&found, "smoothstep"), "{found:?}");
}

#[test]
fn printing_is_refused_because_there_is_nowhere_to_print() {
    let found = errors(
        "function fragment(uv)
           print(uv.x)
           return vec4(1.0)
         end",
    );
    assert!(!found.is_empty());
}

#[test]
fn an_undeclared_name_says_what_a_shader_can_see() {
    let found = errors(
        "function fragment(uv)
           return vec4(mystery)
         end",
    );
    assert!(mentions(&found, "not defined here"), "{found:?}");
    assert!(mentions(&found, "inputs"), "{found:?}");
}

#[test]
fn strings_and_nil_are_refused() {
    assert!(mentions(
        &errors(
            "function fragment(uv)
               local s = \"red\"
               return vec4(1.0)
             end"
        ),
        "no strings"
    ));
    assert!(mentions(
        &errors(
            "function fragment(uv)
               local n = nil
               return vec4(1.0)
             end"
        ),
        "no `nil`"
    ));
}

#[test]
fn a_swizzle_cannot_mix_coordinate_and_colour_names() {
    let found = errors(
        "function fragment(uv)
           local c = vec4(1.0)
           return vec4(c.xg, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "mixes xyzw with rgba"), "{found:?}");
}

#[test]
fn a_component_the_vector_does_not_have_is_named() {
    let found = errors(
        "function fragment(uv)
           return vec4(uv.z, 0.0, 0.0, 1.0)
         end",
    );
    assert!(mentions(&found, "no `z` component"), "{found:?}");
}

#[test]
fn assigning_a_different_type_is_refused() {
    let found = errors(
        "function fragment(uv)
           local d = uv.x
           d = vec3(1.0)
           return vec4(d)
         end",
    );
    assert!(mentions(&found, "holds f32"), "{found:?}");
}

#[test]
fn a_wrong_constructor_width_says_how_many_were_given() {
    let found = errors(
        "function fragment(uv)
           return vec4(1.0, 2.0)
         end",
    );
    assert!(mentions(&found, "needs 4 components"), "{found:?}");
    assert!(mentions(&found, "2 were given"), "{found:?}");
}

#[test]
fn a_shader_that_can_fall_off_the_end_is_refused() {
    // A branch with no else is a path to the end of the function, and WGSL
    // will not accept a value-returning function that reaches it.
    let found = errors(
        "function fragment(uv)
           if uv.x > 0.5 then
             return vec4(1.0)
           end
         end",
    );
    assert!(mentions(&found, "without returning"), "{found:?}");
}

#[test]
fn a_generic_for_points_at_the_numeric_one() {
    let found = errors(
        "function fragment(uv)
           for k, v in pairs(uv) do
           end
           return vec4(1.0)
         end",
    );
    assert!(mentions(&found, "no `for ... in`"), "{found:?}");
    assert!(mentions(&found, "for i = 1, n"), "{found:?}");
}

#[test]
fn a_non_constant_loop_step_explains_why() {
    let found = errors(
        "function fragment(uv)
           for i = 1, 8, uv.x do
           end
           return vec4(1.0)
         end",
    );
    assert!(mentions(&found, "must be a constant"), "{found:?}");
    assert!(mentions(&found, "sign decides"), "{found:?}");
}

#[test]
fn and_or_on_numbers_points_at_select() {
    // `a or b` as a value-picking idiom has no meaning without `nil`, and an
    // author reaching for it needs to be told what to reach for instead.
    let found = errors(
        "function fragment(uv)
           local d = uv.x or 1.0
           return vec4(d)
         end",
    );
    assert!(mentions(&found, "select("), "{found:?}");
}

#[test]
fn break_outside_a_loop_is_refused() {
    let found = errors(
        "function fragment(uv)
           break
         end",
    );
    assert!(mentions(&found, "outside a loop"), "{found:?}");
}

#[test]
fn every_mistake_is_reported_in_one_run() {
    // Diagnostics accumulate: an author fixing three errors should see three,
    // not discover the second after fixing the first.
    let found = errors(
        "function fragment(uv)
           local a = {}
           local b = nil
           local c = \"red\"
           return vec4(1.0)
         end",
    );
    assert!(found.len() >= 3, "expected three, got {found:?}");
}

#[test]
fn a_syntax_error_survives_as_a_diagnostic() {
    let found = errors("function fragment(uv) return vec4( end");
    assert!(!found.is_empty());
}

#[test]
fn texture_outside_an_effect_shader_says_which_kind_to_use() {
    let found = errors(
        "function fragment(uv)
           return texture(uv)
         end",
    );
    assert!(mentions(&found, "effect shader"), "{found:?}");
}
