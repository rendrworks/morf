struct Viewport {
    size: vec2<f32>,
    _padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> viewport: Viewport;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) radii: vec4<f32>,
    @location(4) border: vec4<f32>,
    @location(5) border_color: vec4<f32>,
    @location(6) shape: vec4<f32>,
    @location(7) effects: vec4<f32>,
    @location(8) shadow: vec4<f32>,
    @location(9) shadow_color: vec4<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex: u32,
    @location(0) bounds: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) radii: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) border_color: vec4<f32>,
    @location(5) shape: vec4<f32>,
    @location(6) effects: vec4<f32>,
    @location(7) shadow: vec4<f32>,
    @location(8) shadow_color: vec4<f32>,
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
    let pixel = bounds.xy + corner * bounds.zw;
    let clip = vec2<f32>(
        pixel.x / viewport.size.x * 2.0 - 1.0,
        1.0 - pixel.y / viewport.size.y * 2.0,
    );
    var output: VertexOutput;
    output.position = vec4<f32>(clip, 0.0, 1.0);
    output.local = corner * bounds.zw;
    output.size = bounds.zw;
    output.color = color;
    output.radii = radii;
    output.border = border;
    output.border_color = border_color;
    output.shape = shape;
    output.effects = effects;
    output.shadow = shadow;
    output.shadow_color = shadow_color;
    return output;
}

fn rounded_distance(point: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    let centered = point - size * 0.5;
    let half_size = max(size * 0.5 - vec2<f32>(radius), vec2<f32>(0.0));
    let offset = abs(centered) - half_size;
    return length(max(offset, vec2<f32>(0.0))) + min(max(offset.x, offset.y), 0.0) - radius;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let radius = max(input.radii.x, 0.0);
    let point = input.local - input.shape.xy;
    let distance = rounded_distance(point, input.shape.zw, radius);
    let softness = max(input.effects.x, 0.5);
    let coverage = smoothstep(softness, -softness, distance);
    let border_width = max(input.border.x, 0.0);
    let inner = smoothstep(softness, -softness, distance + border_width);
    let fill_alpha = input.color.a * inner;
    let border_alpha = input.border_color.a * max(coverage - inner, 0.0);
    let shape = vec4<f32>(
        input.color.rgb * fill_alpha + input.border_color.rgb * border_alpha,
        fill_alpha + border_alpha,
    );
    let spread = input.effects.z;
    let shadow_point = point - input.shadow.xy + vec2<f32>(spread);
    let shadow_size = max(input.shape.zw + vec2<f32>(spread * 2.0), vec2<f32>(0.0));
    let shadow_distance = rounded_distance(shadow_point, shadow_size, max(radius + spread, 0.0));
    let shadow_softness = max(input.effects.y, 0.5);
    let shadow_alpha = input.shadow_color.a * smoothstep(shadow_softness, -shadow_softness, shadow_distance);
    let shadow_layer = vec4<f32>(input.shadow_color.rgb * shadow_alpha, shadow_alpha);
    return shape + shadow_layer * (1.0 - shape.a);
}
