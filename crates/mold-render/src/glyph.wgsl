struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) color_overlay: vec4<f32>,
    @location(3) mode: vec4<f32>,
    @location(4) surface_point: vec2<f32>,
    @location(5) mask_bounds: vec4<f32>,
    @location(6) mask_inverse_0: vec4<f32>,
    @location(7) mask_inverse_1: vec4<f32>,
    @location(8) mask_radii: vec4<f32>,
}

@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;

@vertex
fn vs_main(
    @builtin(vertex_index) vertex: u32,
    @location(0) origin: vec2<f32>,
    @location(1) axes: vec4<f32>,
    @location(2) uv_bounds: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) color_overlay: vec4<f32>,
    @location(5) mode: vec4<f32>,
    @location(6) surface: vec4<f32>,
    @location(7) mask_bounds: vec4<f32>,
    @location(8) mask_inverse_0: vec4<f32>,
    @location(9) mask_inverse_1: vec4<f32>,
    @location(10) mask_radii: vec4<f32>,
) -> VertexOutput {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex];
    var output: VertexOutput;
    output.position = vec4<f32>(origin + corner.x * axes.xy + corner.y * axes.zw, 0.0, 1.0);
    output.uv = uv_bounds.xy + corner * uv_bounds.zw;
    output.color = color;
    output.color_overlay = color_overlay;
    output.mode = mode;
    output.surface_point = surface.xy + corner * surface.zw;
    output.mask_bounds = mask_bounds;
    output.mask_inverse_0 = mask_inverse_0;
    output.mask_inverse_1 = mask_inverse_1;
    output.mask_radii = mask_radii;
    return output;
}

fn rounded_distance(point: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    var radius = radii.x;
    if point.y >= size.y * 0.5 {
        radius = select(radii.w, radii.z, point.x >= size.x * 0.5);
    } else {
        radius = select(radii.x, radii.y, point.x >= size.x * 0.5);
    }
    let centered = point - size * 0.5;
    let half_size = max(size * 0.5 - vec2<f32>(radius), vec2<f32>(0.0));
    let offset = abs(centered) - half_size;
    return length(max(offset, vec2<f32>(0.0))) + min(max(offset.x, offset.y), 0.0) - radius;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var sampled = textureSample(atlas, atlas_sampler, input.uv);
    if input.mode.y > 0.5 {
        let local = vec2<f32>(
            dot(input.mask_inverse_0.xyz, vec3<f32>(input.surface_point, 1.0)),
            dot(input.mask_inverse_1.xyz, vec3<f32>(input.surface_point, 1.0)),
        );
        let distance = rounded_distance(
            local - input.mask_bounds.xy,
            input.mask_bounds.zw,
            input.mask_radii,
        );
        let coverage = smoothstep(max(fwidth(distance), 0.0001), -max(fwidth(distance), 0.0001), distance);
        sampled *= coverage;
    }
    let alpha = sampled.a * input.color.a;
    if input.mode.x > 0.5 {
        return vec4<f32>(sampled.rgb * input.color.a, alpha);
    }
    let color = sampled.rgb * input.color.rgb;
    return vec4<f32>(mix(color, input.color_overlay.rgb, input.color_overlay.a) * alpha, alpha);
}
