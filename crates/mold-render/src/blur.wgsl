@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct BlurParams {
    texel: vec2<f32>,
    offset: f32,
    mode: f32,
}

@group(0) @binding(2) var<uniform> params: BlurParams;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex];
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let step = params.texel * params.offset;
    if params.mode < 0.5 {
        var color = textureSample(source, source_sampler, input.uv) * 4.0;
        color += textureSample(source, source_sampler, input.uv + vec2<f32>(-step.x, -step.y));
        color += textureSample(source, source_sampler, input.uv + vec2<f32>(step.x, -step.y));
        color += textureSample(source, source_sampler, input.uv + vec2<f32>(-step.x, step.y));
        color += textureSample(source, source_sampler, input.uv + vec2<f32>(step.x, step.y));
        return color / 8.0;
    }
    var color = textureSample(source, source_sampler, input.uv + vec2<f32>(-step.x, -step.y));
    color += textureSample(source, source_sampler, input.uv + vec2<f32>(step.x, -step.y));
    color += textureSample(source, source_sampler, input.uv + vec2<f32>(-step.x, step.y));
    color += textureSample(source, source_sampler, input.uv + vec2<f32>(step.x, step.y));
    color += textureSample(source, source_sampler, input.uv + vec2<f32>(-step.x * 2.0, 0.0)) * 2.0;
    color += textureSample(source, source_sampler, input.uv + vec2<f32>(step.x * 2.0, 0.0)) * 2.0;
    color += textureSample(source, source_sampler, input.uv + vec2<f32>(0.0, -step.y * 2.0)) * 2.0;
    color += textureSample(source, source_sampler, input.uv + vec2<f32>(0.0, step.y * 2.0)) * 2.0;
    return color / 12.0;
}
