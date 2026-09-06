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
    @location(11) morph_uv: vec2<f32>,
    @location(12) morph_size: vec2<f32>,
    @location(13) ramp: f32,
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
    @location(13) morph_bounds: vec4<f32>,
    @location(14) ramp: f32,
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
    output.morph_uv = morph_bounds.xy + corner * morph_bounds.zw;
    output.morph_size = morph_bounds.zw;
    output.ramp = ramp;
    return output;
}


/// What a configuration's effect shader reads when it samples underneath.
///
/// The contract a compiled shader is built against: it calls `morf_sample` and
/// this pass decides what that means. For a composited layer it is the layer's
/// own rendered content, which is what makes a distortion or a custom blur
/// possible at all.
fn morf_sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(atlas, atlas_sampler, uv);
}

/// Where a configuration's effect shader gets to rework what was rendered.
///
/// The default hands the sample straight back, so an ordinary layer composite
/// costs nothing. Attaching an effect shader replaces this body with a call
/// into the compiled one.
fn morf_effect_hook(uv: vec2<f32>, sampled: vec4<f32>) -> vec4<f32> {
    return sampled;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var sampled = morf_effect_hook(input.uv, morf_sample(input.uv));

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
        //
        // A morphing glyph arrives as two neighbouring frames of the journey
        // rather than as its two end letters. The correspondence between the
        // letters — which stroke becomes which — was solved in the outline
        // before either was measured, so these two shapes differ by a twelfth
        // of the way across and averaging them has almost nothing to get wrong.
        let edge = input.field.x;
        // Half the field's change across one device pixel. Taken from the size
        // the glyph is drawn at rather than measured from the sampled field:
        // `fwidth` of a minified texture varies from pixel to pixel, and an
        // antialiasing width that wobbles is what a hard, unsettled edge looks
        // like. Floored at the field's own precision, because a byte cannot
        // resolve an edge finer than one step in two hundred and fifty-five.
        const FIELD_STEP: f32 = 1.0 / 255.0;
        let feather = max(input.ramp * 0.5, FIELD_STEP) + input.field.y;

        // Four readings inside the pixel rather than one at its centre.
        //
        // A single reading thresholded over one pixel is the right *coverage*
        // and still looks stepped: it gives one intermediate value per pixel,
        // so along a shallow curve — where the edge moves a fraction of a pixel
        // per column — whole runs of pixels come out the same and the eye reads
        // the run as a stair. Four readings on a rotated grid, averaged,
        // approximate how much of the pixel the letter actually covers, which
        // is what a rasterizer computes exactly and what the stair was missing.
        //
        // The offsets are in pixels, turned into atlas coordinates by how far a
        // pixel reaches across the atlas here — which the derivatives already
        // know, and which is right whatever size the glyph is drawn at.
        let across = dpdx(input.uv);
        let down = dpdy(input.uv);
        let morph_across = dpdx(input.morph_uv);
        let morph_down = dpdy(input.morph_uv);
        let morphing = input.morph_size.x > 0.0 && input.morph_size.y > 0.0;
        // A rotated grid, so neither a horizontal nor a vertical edge has two
        // readings land on the same line as each other.
        var taps = array<vec2<f32>, 4>(
            vec2<f32>(-0.125, -0.375),
            vec2<f32>(0.375, -0.125),
            vec2<f32>(-0.375, 0.125),
            vec2<f32>(0.125, 0.375),
        );
        var fill = 0.0;
        var outer = 0.0;
        for (var index = 0; index < 4; index = index + 1) {
            let tap = taps[index];
            let shift = across * tap.x + down * tap.y;
            var here = textureSample(atlas, atlas_sampler, input.uv + shift).r;
            if morphing {
                let partner = textureSample(
                    atlas,
                    atlas_sampler,
                    input.morph_uv + morph_across * tap.x + morph_down * tap.y,
                ).r;
                here = mix(here, partner, input.field.w);
            }
            fill = fill + (1.0 - smoothstep(edge - feather, edge + feather, here));
            outer = outer + (1.0 - smoothstep(
                edge + input.field.z - feather,
                edge + input.field.z + feather,
                here,
            ));
        }
        fill = fill * 0.25;
        outer = outer * 0.25;
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
    // A solid quad — a decoration line — takes no coverage from the atlas.
    let sampled_alpha = select(
        select(sampled.a, sampled.r, input.mode.z > 0.5),
        mask_coverage,
        input.mode.z > 1.5,
    );
    let alpha = sampled_alpha * input.color.a;
    if input.mode.x > 0.5 {
        return vec4<f32>(sampled.rgb * input.color.a, alpha);
    }
    let color = select(sampled.rgb * input.color.rgb, input.color.rgb, input.mode.z > 0.5);
    return vec4<f32>(mix(color, input.color_overlay.rgb, input.color_overlay.a) * alpha, alpha);
}
