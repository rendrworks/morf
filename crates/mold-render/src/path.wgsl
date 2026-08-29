struct Viewport {
    size: vec2<f32>,
    padding: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> viewport: Viewport;

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
) -> VertexOutput {
    let clip = vec2<f32>(
        position.x / viewport.size.x * 2.0 - 1.0,
        1.0 - position.y / viewport.size.y * 2.0,
    );
    var output: VertexOutput;
    output.position = vec4<f32>(clip, 0.0, 1.0);
    output.color = color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color.rgb * input.color.a, input.color.a);
}
