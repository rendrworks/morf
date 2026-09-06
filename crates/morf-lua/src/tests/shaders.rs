use super::*;

// `morf.shader` end to end through a configuration: registration, compilation,
// attachment, and the two things the compiler derives that nothing declares.

#[test]
fn a_configuration_registers_and_attaches_a_shader() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "shader.lua",
            r#"
            local morf = require("morf")
            local ui = require("morf.ui")
            morf.shader("tint", {
              params = { level = 0.5 },
              fragment = [[
                function fragment(uv, time, resolution, coverage, level)
                  return vec4(level, uv.x, 0.0, 1.0)
                end
              ]],
            })
            ui.Item {
              ui.Rect { width = 10, height = 10, shader = "tint", shader_params = { level = 0.25 } },
            }
            "#
            .as_bytes(),
        )
        .expect("the configuration loads");

    let shaders = runtime.shaders();
    assert_eq!(shaders.len(), 1, "one program was registered");
    assert!(
        shaders[0].wgsl.contains("morf_shader_main"),
        "and it produced a shader entry point",
    );
    // The parameter reached its declared offset, after the frame's own values.
    assert_eq!(shaders[0].offsets, vec![morf_shader::HEADER_BYTES]);
}

#[test]
fn reading_the_clock_is_what_decides_whether_a_shader_repaints() {
    let still = shader_runtime(
        "function fragment(uv)
           return vec4(uv.x, 0.0, 0.0, 1.0)
         end",
    );
    assert!(
        !still.shaders_animate(),
        "a shader that ignores time does not force a repaint",
    );

    let moving = shader_runtime(
        "function fragment(uv, time)
           return vec4(sin(time), 0.0, 0.0, 1.0)
         end",
    );
    assert!(
        moving.shaders_animate(),
        "and one that reads it repaints every frame",
    );
}

#[test]
fn a_shader_that_does_not_compile_fails_the_configuration() {
    // The author is watching the terminal now; they will not be watching when
    // the node first becomes visible.
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "shader.lua",
            r#"
            local morf = require("morf")
            morf.shader("broken", {
              fragment = [[
                function fragment(uv)
                  if uv.x then
                    return vec4(1.0)
                  end
                  return vec4(0.0)
                end
              ]],
            })
            "#
            .as_bytes(),
        )
        .expect_err("a bad shader stops the configuration");
    let message = format!("{error}");
    assert!(message.contains("must be a bool"), "{message}");
    assert!(message.contains("x > 0.0"), "{message}");
}

#[test]
fn attaching_a_shader_nobody_registered_says_so() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "shader.lua",
            r#"
            local ui = require("morf.ui")
            ui.Item { ui.Rect { width = 10, height = 10, shader = "missing" } }
            "#
            .as_bytes(),
        )
        .expect_err("an unknown shader is an error");
    assert!(format!("{error}").contains("never registered"));
}

fn shader_runtime(body: &str) -> Runtime {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "shader.lua",
            format!(
                r#"
                local morf = require("morf")
                morf.shader("probe", {{ fragment = [[{body}]] }})
                "#
            )
            .as_bytes(),
        )
        .expect("the configuration loads");
    runtime
}

#[test]
fn a_configuration_fills_a_data_block() {
    // The whole §5.3 path from Lua: declare a block, attach the shader, push
    // numbers into it. Tested here rather than only in the renderer because
    // this is the half a configuration author touches.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "levels.lua",
            r#"
            local morf = require("morf")
            local ui = require("morf.ui")
            morf.shader("bars", {
              data = { levels = 4 },
              fragment = [[
                function fragment(uv, time, resolution, coverage)
                  local slot = clamp(i32(uv.x * 4.0), 0, 3)
                  return vec4(levels[slot], 0.0, 0.0, 1.0)
                end
              ]],
            })
            local node = ui.Rect { width = 40, height = 10, shader = "bars" }
            morf.shader_data(node, "levels", { 0.25, 0.5, 0.75, 1.0 })
            ui.Item { node }
            "#
            .as_bytes(),
        )
        .expect("the configuration loads");

    let shaders = runtime.shaders();
    assert_eq!(shaders[0].data, vec![("levels".to_owned(), 4)]);
}

#[test]
fn pushing_to_a_block_that_does_not_exist_says_so() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "levels.lua",
            r#"
            local morf = require("morf")
            local ui = require("morf.ui")
            morf.shader("bars", {
              data = { levels = 2 },
              fragment = [[
                function fragment(uv, time, resolution, coverage)
                  return vec4(levels[0], 0.0, 0.0, 1.0)
                end
              ]],
            })
            local node = ui.Rect { width = 40, height = 10, shader = "bars" }
            morf.shader_data(node, "mystery", { 1.0 })
            ui.Item { node }
            "#
            .as_bytes(),
        )
        .expect_err("an unknown block is an error");
    assert!(format!("{error}").contains("no data block"));
}

#[test]
fn a_vertex_shader_is_compiled_beside_the_fragment_one() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "sway.lua",
            r#"
            local morf = require("morf")
            morf.shader("sway", {
              vertex = [[
                function vertex(corner, size, time)
                  return corner + vec2(sin(time) * 4.0, 0.0)
                end
              ]],
              fragment = [[
                function fragment(uv, time, resolution, coverage)
                  return vec4(uv.x, uv.y, 0.5, 1.0)
                end
              ]],
            })
            "#
            .as_bytes(),
        )
        .expect("the configuration loads");

    let shaders = runtime.shaders();
    let vertex = shaders[0].vertex.as_ref().expect("a vertex stage");
    assert!(vertex.contains("-> vec2<f32>"), "{vertex}");
    // The vertex stage reads the clock, so the node repaints even though the
    // fragment stage does not — which is what `shaders_animate` has to notice.
    assert!(runtime.shaders_animate(), "a swaying node repaints");
}

#[test]
fn a_texture_a_configuration_declares_is_carried_to_the_host() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "masked.lua",
            r#"
            local morf = require("morf")
            morf.shader("masked", {
              textures = { mask = "/tmp/morf-mask.png" },
              fragment = [[
                function fragment(uv, time, resolution, coverage)
                  return texture(mask, uv)
                end
              ]],
            })
            "#
            .as_bytes(),
        )
        .expect("the configuration loads");

    let shaders = runtime.shaders();
    assert_eq!(shaders[0].textures, vec!["/tmp/morf-mask.png"]);
}
