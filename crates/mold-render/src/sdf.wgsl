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
    @location(10) gradient_start_color: vec4<f32>,
    @location(11) gradient_end_color: vec4<f32>,
    @location(12) gradient_points: vec4<f32>,
    @location(13) gradient_data: vec4<f32>,
    @location(14) color_overlay: vec4<f32>,
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
    @location(9) gradient_start_color: vec4<f32>,
    @location(10) gradient_end_color: vec4<f32>,
    @location(11) gradient_points: vec4<f32>,
    @location(12) gradient_data: vec4<f32>,
    @location(13) color_overlay: vec4<f32>,
    @location(14) transform: vec4<f32>,
    @location(15) transform_offset: vec2<f32>,
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
    let source = bounds.xy + corner * bounds.zw;
    let pixel = vec2<f32>(
        transform.x * source.x + transform.z * source.y,
        transform.y * source.x + transform.w * source.y,
    ) + transform_offset;
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
    output.gradient_start_color = gradient_start_color;
    output.gradient_end_color = gradient_end_color;
    output.gradient_points = gradient_points;
    output.gradient_data = gradient_data;
    output.color_overlay = color_overlay;
    return output;
}

fn rounded_distance(point: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    var radius = radii.x;
    if point.y >= size.y * 0.5 {
        radius = select(radii.w, radii.z, point.x >= size.x * 0.5);
    } else {
        radius = select(radii.x, radii.y, point.x >= size.x * 0.5);
    }
    radius = max(radius, 0.0);
    let centered = point - size * 0.5;
    let half_size = max(size * 0.5 - vec2<f32>(radius), vec2<f32>(0.0));
    let offset = abs(centered) - half_size;
    return length(max(offset, vec2<f32>(0.0))) + min(max(offset.x, offset.y), 0.0) - radius;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let point = input.local - input.shape.xy;
    let signed_distance = rounded_distance(point, input.shape.zw, input.radii);
    let softness = max(input.effects.x, 0.5);
    let coverage = smoothstep(softness, -softness, signed_distance);
    let border_width = max(input.border.x, 0.0);
    let inner = smoothstep(softness, -softness, signed_distance + border_width);
    let normalized = point / max(input.shape.zw, vec2<f32>(0.000001));
    var fill_color = input.color;
    if input.gradient_data.x == 1.0 {
        let direction = input.gradient_points.zw - input.gradient_points.xy;
        let amount = dot(normalized - input.gradient_points.xy, direction) / max(dot(direction, direction), 0.000001);
        fill_color = mix(input.gradient_start_color, input.gradient_end_color, clamp(amount, 0.0, 1.0));
    } else if input.gradient_data.x == 2.0 {
        let amount = distance(normalized, input.gradient_data.yz) / max(input.gradient_data.w, 0.000001);
        fill_color = mix(input.gradient_start_color, input.gradient_end_color, clamp(amount, 0.0, 1.0));
    } else if input.gradient_data.x == 3.0 {
        let delta = normalized - input.gradient_data.yz;
        let amount = fract((atan2(delta.y, delta.x) - input.effects.w) / 6.28318530718);
        fill_color = mix(input.gradient_start_color, input.gradient_end_color, amount);
    }
    let fill_alpha = fill_color.a * inner;
    let border_alpha = input.border_color.a * max(coverage - inner, 0.0);
    let shape = vec4<f32>(
        fill_color.rgb * fill_alpha + input.border_color.rgb * border_alpha,
        fill_alpha + border_alpha,
    );
    let spread = input.effects.z;
    let shadow_point = point - input.shadow.xy + vec2<f32>(spread);
    let shadow_size = max(input.shape.zw + vec2<f32>(spread * 2.0), vec2<f32>(0.0));
    let shadow_distance = rounded_distance(shadow_point, shadow_size, max(input.radii + vec4<f32>(spread), vec4<f32>(0.0)));
    let shadow_softness = max(input.effects.y, 0.5);
    let outer_shadow = select(1.0, 0.0, input.shadow.z > 0.5);
    let shadow_alpha = input.shadow_color.a * outer_shadow * smoothstep(shadow_softness, -shadow_softness, shadow_distance);
    let shadow_layer = vec4<f32>(input.shadow_color.rgb * shadow_alpha, shadow_alpha);
    let inner_point = point - input.shadow.xy;
    let inner_distance = rounded_distance(inner_point, input.shape.zw, input.radii);
    let inner_amount = input.shadow_color.a * coverage * smoothstep(-shadow_softness, shadow_softness, inner_distance) * input.shadow.z;
    let outer_result = shape + shadow_layer * (1.0 - shape.a);
    let result = vec4<f32>(
        mix(outer_result.rgb, input.shadow_color.rgb * outer_result.a, inner_amount),
        outer_result.a,
    );
    return vec4<f32>(
        mix(result.rgb, input.color_overlay.rgb * result.a, input.color_overlay.a),
        result.a,
    );
}
