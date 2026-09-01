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
