@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct BlurParams {
    texel: vec2<f32>,
    offset: f32,
    mode: f32,
}

@group(0) @binding(2) var<uniform> params: BlurParams;

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
