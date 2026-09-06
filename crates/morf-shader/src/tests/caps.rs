use super::*;

// Every cap produces its named diagnostic and never a panic. A shader that
// exceeds one is a configuration mistake, and the blast radius of getting this
// wrong is the user's whole session, so each is tested rather than trusted.

#[test]
fn an_oversized_source_is_refused_by_length() {
    let body = format!(
        "function fragment(uv)\n{}\n  return vec4(1.0)\nend",
        "  -- padding\n".repeat(MAX_SOURCE_BYTES / 12 + 16)
    );
    let found = match compile_material(&body) {
        Ok(_) => panic!("expected the length cap to fire"),
        Err(diagnostics) => diagnostics,
    };
    assert!(mentions(&found, "over the"), "{found:?}");
}

#[test]
fn too_many_parameters_are_refused() {
    let params: Vec<Binding> = (0..MAX_PARAMS + 1)
        .map(|index| Binding {
            name: format!("p{index}"),
            ty: Type::F32,
        })
        .collect();
    let found = compile_with(
        "function fragment(uv)
           return vec4(1.0)
         end",
        ShaderKind::Material,
        params,
    )
    .expect_err("expected the parameter cap to fire");
    assert!(mentions(&found, "parameters"), "{found:?}");
}

#[test]
fn loops_nested_past_the_limit_are_refused() {
    let mut body = String::from("function fragment(uv)\n  local t = 0.0\n");
    for _ in 0..MAX_LOOP_NESTING + 1 {
        body.push_str("  while t < 1.0 do\n");
    }
    body.push_str("  t = t + 1.0\n");
    for _ in 0..MAX_LOOP_NESTING + 1 {
        body.push_str("  end\n");
    }
    body.push_str("  return vec4(t)\nend");
    let found = match compile_material(&body) {
        Ok(_) => panic!("expected the nesting cap to fire"),
        Err(diagnostics) => diagnostics,
    };
    assert!(mentions(&found, "nested deeper"), "{found:?}");
}

#[test]
fn a_shader_at_the_edge_of_the_nesting_limit_still_compiles() {
    // The cap has to reject one more than it allows, not one fewer: an
    // off-by-one here would refuse a shader somebody legitimately wrote.
    let mut body = String::from("function fragment(uv)\n  local t = 0.0\n");
    for _ in 0..MAX_LOOP_NESTING {
        body.push_str("  while t < 1.0 do\n");
    }
    body.push_str("  t = t + 1.0\n");
    for _ in 0..MAX_LOOP_NESTING {
        body.push_str("  end\n");
    }
    body.push_str("  return vec4(t)\nend");
    assert!(compile_material(&body).is_ok());
}
