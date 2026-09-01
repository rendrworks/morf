use super::*;

// Shaders ported from Shadertoy, by way of the RbxShader collection.
//
// This is the honest measure of whether the subset is expressive enough. A
// shader language nobody can port a real shader into is a checkbox, and the
// only way to find out is to take shaders somebody else wrote and see what they
// need. Everything these asked for that was missing — `atan2`, `tanh`, the rest
// of the inverse trigonometry — was added because they asked for it.

/// Compiles a port, printing the compiler's own diagnostics when it fails.
fn port(name: &str, body: &str) -> Compiled {
    compile_material(body).unwrap_or_else(|errors| panic!("{name}:\n{}", report(name, &errors)))
}

#[test]
fn plasma_ports() {
    // "Plasma" by @XorDev, shadertoy.com/view/WfS3Dd.
    //
    // Centred aspect-corrected coordinates, a fluid field advanced by a loop,
    // and a `tanh` tonemap. The loop's trip count is fixed but its body reads
    // what the previous pass wrote, which a traced design could have handled;
    // the `abs(f.x - f.y)` line brightness and the vector division could not
    // have been guessed from types alone.
    let compiled = port(
        "plasma",
        r#"
        function fragment(uv, time, resolution)
          local I = uv * resolution
          local r = resolution
          local p = (I + I - r) / r.y
          local z = 4.0
          local O = vec3(0.0, 0.0, 0.0)
          local f = p * (z - 4.0 * abs(0.7 - dot(p, p)))
          local i = 0.0
          while i < 8.0 do
            i = i + 1.0
            local s = sin(f) + vec2(1.0, 1.0)
            O = O + vec3(s.x, s.y, s.y) * abs(f.x - f.y)
            f = f + cos(f.yx * i + vec2(i, i) + vec2(time, time)) / i + vec2(0.7, 0.7)
          end
          local shade = vec3(z - 4.0, z - 4.0, z - 4.0) - p.y * vec3(-1.0, 1.0, 2.0)
          O = tanh(7.0 * exp(shade) / O)
          return vec4(O, 1.0)
        end
        "#,
    );
    assert!(compiled.reads_time, "plasma animates");
    assert!(compiled.wgsl.contains("tanh("), "the tonemap survived");
}

#[test]
fn a_polar_rosette_ports() {
    // The shape that made `atan2` non-negotiable: without it there is no way to
    // get an angle out of a coordinate, and every polar shader starts there.
    let compiled = port(
        "rosette",
        r#"
        function fragment(uv, time, resolution)
          local p = uv - vec2(0.5, 0.5)
          local angle = atan2(p.y, p.x)
          local radius = length(p)
          local petal = cos(angle * 6.0) * 0.15 + 0.3
          local edge = smoothstep(petal + 0.01, petal - 0.01, radius)
          return vec4(edge, edge * 0.4, 1.0 - edge, 1.0)
        end
        "#,
    );
    assert!(compiled.wgsl.contains("atan2("));
}

#[test]
fn a_raymarcher_ports_with_a_real_early_exit() {
    // The case a tracer could not have handled, and the reason this is a
    // compiler. The loop breaks on a distance the loop itself computed: a
    // traced design would have unrolled all sixty-four steps unconditionally,
    // paying the worst case on every pixel, because the exit is data the trace
    // does not have.
    let compiled = port(
        "raymarch",
        r#"
        function fragment(uv, time, resolution)
          local p = (uv - vec2(0.5, 0.5)) * 2.0
          local origin = vec3(0.0, 0.0, -3.0)
          local ray = normalize(vec3(p.x, p.y, 1.5))
          local travelled = 0.0
          local hit = 0.0
          local step_index = 0.0
          while step_index < 64.0 do
            step_index = step_index + 1.0
            local at = origin + ray * travelled
            local wobble = sin(at.x * 2.0 + time) * 0.15
            local distance = length(at) - (1.0 + wobble)
            if distance < 0.001 then
              hit = 1.0
              step_index = 64.0
            end
            if travelled > 20.0 then
              step_index = 64.0
            end
            travelled = travelled + distance
          end
          local shade = 1.0 - clamp(travelled / 8.0, 0.0, 1.0)
          return vec4(hit * shade, hit * shade * 0.5, hit, 1.0)
        end
        "#,
    );
    assert!(compiled.reads_time);
    // The guard is still there. An early exit the shader controls does not
    // remove the ceiling it cannot control.
    assert!(compiled.wgsl.contains("morf_guard0 >= "));
}

#[test]
fn a_raymarcher_can_break_out_of_its_loop() {
    // The same thing written the way somebody would actually write it. `break`
    // exists, and it is what makes the port a translation rather than a
    // rewrite.
    let compiled = port(
        "raymarch-break",
        r#"
        function fragment(uv, time, resolution)
          local p = (uv - vec2(0.5, 0.5)) * 2.0
          local ray = normalize(vec3(p.x, p.y, 1.5))
          local travelled = 0.0
          local i = 0.0
          while i < 48.0 do
            i = i + 1.0
            local at = vec3(0.0, 0.0, -3.0) + ray * travelled
            local d = length(at) - 1.0
            if d < 0.001 then
              break
            end
            travelled = travelled + d
          end
          local shade = 1.0 - clamp(travelled / 8.0, 0.0, 1.0)
          return vec4(shade, shade, shade, 1.0)
        end
        "#,
    );
    assert!(compiled.wgsl.contains("break;"));
}

#[test]
fn a_scanline_and_grade_effect_ports() {
    // The shape most shell shaders actually take: no control flow, some
    // arithmetic over `uv` and the clock, and a grade at the end.
    port(
        "scanlines",
        r#"
        function fragment(uv, time, resolution)
          local scan = sin(uv.y * resolution.y * 1.6 + time * 6.0) * 0.5 + 0.5
          local vignette = 1.0 - length(uv - vec2(0.5, 0.5)) * 0.9
          local base = mix(vec3(0.05, 0.07, 0.1), vec3(0.2, 0.9, 0.7), uv.x)
          local graded = base * (0.85 + scan * 0.15) * clamp(vignette, 0.0, 1.0)
          return vec4(graded, 1.0)
        end
        "#,
    );
}

#[test]
fn value_noise_ports() {
    // Noise without a random number generator: a hash of the coordinate, which
    // is what the diagnostic for `math.random` points people at.
    port(
        "noise",
        r#"
        function fragment(uv, time, resolution)
          local p = uv * 8.0
          local cell = floor(p)
          local within = fract(p)
          local smoothed = within * within * (vec2(3.0, 3.0) - 2.0 * within)
          local a = fract(sin(dot(cell, vec2(127.1, 311.7))) * 43758.5453)
          local b = fract(sin(dot(cell + vec2(1.0, 0.0), vec2(127.1, 311.7))) * 43758.5453)
          local c = fract(sin(dot(cell + vec2(0.0, 1.0), vec2(127.1, 311.7))) * 43758.5453)
          local d = fract(sin(dot(cell + vec2(1.0, 1.0), vec2(127.1, 311.7))) * 43758.5453)
          local top = mix(a, b, smoothed.x)
          local bottom = mix(c, d, smoothed.x)
          local value = mix(top, bottom, smoothed.y)
          return vec4(value, value, value, 1.0)
        end
        "#,
    );
}

#[test]
fn shader_art_ports_with_its_palette_helper() {
    // "ShaderArt" from the RbxShader collection, which uses iq's palette
    // function — `a + b*cos(6.28318*(c*t+d))`. A helper is the single most
    // common idiom in the shaders people want to port, and a language without
    // one makes every port a rewrite. The helper's parameter type comes from
    // the call, because Lua has nowhere to declare it.
    let compiled = port(
        "shader-art",
        r#"
        function palette(t)
          local a = vec3(0.5, 0.5, 0.5)
          local b = vec3(0.5, 0.5, 0.5)
          local c = vec3(1.0, 1.0, 1.0)
          local d = vec3(0.263, 0.416, 0.557)
          return a + b * cos(6.28318 * (c * t + d))
        end

        function fragment(uv, time, resolution)
          local coords = uv * resolution
          local p = (coords * 2.0 - resolution) / resolution.y
          local p0 = p
          local total = vec3(0.0, 0.0, 0.0)
          for i = 0, 3 do
            p = fract(p * 1.5) - vec2(0.5, 0.5)
            local d = length(p) * exp(0.0 - length(p0))
            local col = palette(length(p0) + i * 0.4 + time * 0.4)
            d = sin(d * 8.0 + time) / 8.0
            d = abs(d)
            d = pow(0.01 / d, 1.2)
            total = total + col * d
          end
          return vec4(total, 1.0)
        end
        "#,
    );
    assert!(
        compiled.wgsl.contains("fn morf_fn_palette_0("),
        "{}",
        compiled.wgsl
    );
    assert!(
        compiled.wgsl.contains("morf_fn_palette_0("),
        "and it is called"
    );
}

#[test]
fn a_helper_is_monomorphised_per_argument_type() {
    // The honest reading of an untyped signature: one helper called at two
    // types is two functions, not one that tries to be both. Inferring a single
    // type would have to pick a loser.
    let compiled = port(
        "twice",
        r#"
        function double(x)
          return x + x
        end

        function fragment(uv, time, resolution)
          local a = double(uv.x)
          local b = double(vec3(0.1, 0.2, 0.3))
          return vec4(b * a, 1.0)
        end
        "#,
    );
    assert!(
        compiled.wgsl.contains("morf_fn_double_0"),
        "{}",
        compiled.wgsl
    );
    assert!(compiled.wgsl.contains("morf_fn_double_1"), "two instances");
    assert!(compiled.wgsl.contains("x_1: f32"), "one at f32");
    assert!(compiled.wgsl.contains("x_2: vec3<f32>"), "one at vec3");
}

#[test]
fn a_helper_that_calls_itself_is_refused() {
    // Recursion has no bottom to monomorphise towards, and a shader has no
    // stack to run it on. Caught rather than looping forever in the compiler.
    let found = errors(
        r#"
        function spiral(x)
          return spiral(x - 1.0)
        end

        function fragment(uv)
          return vec4(spiral(4.0), 0.0, 0.0, 1.0)
        end
        "#,
    );
    assert!(mentions(&found, "calls itself"), "{found:?}");
    assert!(
        mentions(&found, "write the repetition as a loop"),
        "{found:?}"
    );
}

#[test]
fn a_helper_cannot_reach_the_callers_locals() {
    // A helper sees its own parameters and nothing else. Leaking the caller's
    // scope would make a shader's meaning depend on where it was called from.
    let found = errors(
        r#"
        function helper(x)
          return x * secret
        end

        function fragment(uv)
          local secret = 2.0
          return vec4(helper(1.0), 0.0, 0.0, 1.0)
        end
        "#,
    );
    assert!(mentions(&found, "not defined here"), "{found:?}");
}

#[test]
fn a_helper_cannot_shadow_a_builtin() {
    // Shadowing `sin` would be a trap rather than a feature: a reader has no
    // way to tell which one a call means.
    let compiled = port(
        "shadow",
        r#"
        function sin(x)
          return x * 100.0
        end

        function fragment(uv)
          return vec4(sin(0.5), 0.0, 0.0, 1.0)
        end
        "#,
    );
    assert!(compiled.wgsl.contains("sin(0.5)"), "the builtin won");
    assert!(!compiled.wgsl.contains("100.0"), "the helper was not used");
}

#[test]
fn cross_products_work_now() {
    // The one builtin the whole RbxShader collection needed that was missing.
    let compiled = port(
        "cross",
        r#"
        function fragment(uv, time, resolution)
          local a = vec3(1.0, 0.0, 0.0)
          local b = vec3(0.0, 1.0, 0.0)
          return vec4(cross(a, b), 1.0)
        end
        "#,
    );
    assert!(compiled.wgsl.contains("cross("));
}
