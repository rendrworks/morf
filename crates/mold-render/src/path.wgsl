struct Viewport {
    size: vec2<f32>,
    padding: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) coverage: f32,
}

@group(0) @binding(0) var<uniform> viewport: Viewport;

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) coverage: f32,
) -> VertexOutput {
    let clip = vec2<f32>(
        position.x / viewport.size.x * 2.0 - 1.0,
        1.0 - position.y / viewport.size.y * 2.0,
    );
    var output: VertexOutput;
    output.position = vec4<f32>(clip, 0.0, 1.0);
    output.color = color;
    output.coverage = coverage;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Interior geometry is marked with a negative position and covers its pixel
    // outright. The band runs from the outline out to one pixel beyond it, so
    // its coverage is simply how far short of that lip the fragment falls.
    var coverage = 1.0;
    if input.coverage > -1.0 {
        coverage = clamp(1.0 - input.coverage, 0.0, 1.0);
    }
    let alpha = input.color.a * coverage;
    return vec4<f32>(input.color.rgb * alpha, alpha);
}
