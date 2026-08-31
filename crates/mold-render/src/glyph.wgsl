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
    @location(9) field: vec4<f32>,
    @location(10) outline_color: vec4<f32>,
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
    @location(11) field: vec4<f32>,
    @location(12) outline_color: vec4<f32>,
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
    output.field = field;
    output.outline_color = outline_color;
    return output;
}


@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var sampled = textureSample(atlas, atlas_sampler, input.uv);

    // A layer mask trims the quad. For a distance field the sampled red channel
    // carries distance rather than coverage, so the mask is applied to the final
    // alpha instead of to the sample it is derived from.
    var mask_coverage = 1.0;
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
        let edge = max(fwidth(distance), 0.0001);
        mask_coverage = smoothstep(edge, -edge, distance);
    }

    if input.mode.w > 0.5 {
        // The field runs from inside to outside across the encoded spread, so
        // the edge is wherever the requested weight sits and a wider outline is
        // simply a second threshold further out.
        let field = sampled.r;
        let feather = max(fwidth(field), 0.0001) + input.field.y;
        let edge = input.field.x;
        let fill = 1.0 - smoothstep(edge - feather, edge + feather, field);
        let outer = 1.0 - smoothstep(
            edge + input.field.z - feather,
            edge + input.field.z + feather,
            field,
        );
        let body = mix(input.color.rgb, input.color_overlay.rgb, input.color_overlay.a);
        let body_alpha = fill * input.color.a;
        // The outline occupies only the band the fill does not, and sits under
        // it, so a translucent fill does not double-darken over its own outline.
        let outline_alpha = max(outer - fill, 0.0) * input.outline_color.a;
        let alpha = (body_alpha + outline_alpha * (1.0 - body_alpha)) * mask_coverage;
        let premultiplied = (body * body_alpha
            + input.outline_color.rgb * outline_alpha * (1.0 - body_alpha)) * mask_coverage;
        return vec4<f32>(premultiplied, alpha);
    }

    sampled *= mask_coverage;
    let sampled_alpha = select(sampled.a, sampled.r, input.mode.z > 0.5);
    let alpha = sampled_alpha * input.color.a;
    if input.mode.x > 0.5 {
        return vec4<f32>(sampled.rgb * input.color.a, alpha);
    }
    let color = select(sampled.rgb * input.color.rgb, input.color.rgb, input.mode.z > 0.5);
    return vec4<f32>(mix(color, input.color_overlay.rgb, input.color_overlay.a) * alpha, alpha);
}
