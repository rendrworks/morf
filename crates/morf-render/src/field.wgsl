// Composed signed-distance fields.
//
// Every layer is a closed-form distance function evaluated per fragment, so an
// edge stays exact at any magnification and two shapes can be interpolated as
// fields rather than as outlines. That is the whole reason for this pass: a
// field morph passes through shapes neither end describes and survives a change
// of topology — one blob splitting into two — which no vertex interpolation
// can express.

struct Uniforms {
    // Logical surface size, and the scale the surface is presented at.
    viewport: vec4<f32>,
};

struct Layer {
    // [shape, morph_to, morph, operation]
    kinds: vec4<f32>,
    // Layer rectangle in the field's own space: centre then half-extents.
    rect: vec4<f32>,
    // [unused, points, inner radius, thickness]
    params: vec4<f32>,
    // [angle, rotation, blend, unused]
    extra: vec4<f32>,
    // Linear-light fill for this layer.
    color: vec4<f32>,
    // Corner radii: top-left, top-right, bottom-right, bottom-left.
    radii: vec4<f32>,
};

/// Everything about a field's surface that is not its shape.
///
/// One per instance, read by `instance_index`, because there is exactly one of
/// these per composed field and threading an index through the vertex
/// attributes to say so would only be a way of getting it wrong. It lives in a
/// storage buffer rather than in attributes because the quad pipeline this
/// pass absorbed already used sixteen of them, and sixteen is the limit.
struct Material {
    // [alignment, antialiased, unused, unused]. Alignment is 0 inside, 1
    // centred, 2 outside; the width itself rides in `style.x`, where the host
    // already needs it to size the quad.
    border: vec4<f32>,
    border_color: vec4<f32>,
    // [offset x, offset y, inner, unused]
    shadow: vec4<f32>,
    shadow_color: vec4<f32>,
    // [unused, shadow blur, shadow spread, conic rotation]
    effects: vec4<f32>,
    gradient_start_color: vec4<f32>,
    gradient_end_color: vec4<f32>,
    // Normalised start and end of a linear gradient.
    gradient_points: vec4<f32>,
    // [kind, centre x, centre y, radius]
    gradient_data: vec4<f32>,
    color_overlay: vec4<f32>,
    // The rectangle a gradient is measured across, in the field's own space:
    // origin then size.
    shape: vec4<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> layers: array<Layer>;
@group(0) @binding(2) var<storage, read> materials: array<Material>;
// The outlines polygon layers walk. One flat buffer for the whole frame; a
// layer says where its own run starts and how long it is.
@group(0) @binding(3) var<storage, read> outline: array<vec2<f32>>;

/// Only `local` varies across the quad. Everything else here is one number per
/// instance, written identically at all four corners — so it is `flat`, and not
/// only to save three interpolators.
///
/// `style.z` and `style.w` are the first layer's index and the layer count,
/// carried as floats because that is what a varying is. Interpolated, a value
/// written as 7.0 at every corner comes back as 6.9999997 in the middle, and
/// `u32()` truncates towards zero: the field then reads the *previous* field's
/// layers and draws nothing anyone asked for. Which fields it hit depended on
/// their layer index, so it looked like shapes going missing at random.
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) fill: vec4<f32>,
    @location(2) @interpolate(flat) outline: vec4<f32>,
    // [stroke width, softness, first layer, layer count]
    @location(3) @interpolate(flat) style: vec4<f32>,
    @location(4) @interpolate(flat) material: u32,
};

/// Where a configuration's own shader gets to move a corner.
///
/// The default returns it untouched, so an ordinary field costs nothing.
/// A vertex shader moves the *quad*, not the shape inside it: the fragment
/// stage still walks the field in the node's own space, so a displaced node
/// keeps its geometry and takes it somewhere else.
fn morf_vertex_hook(corner: vec2<f32>, size: vec2<f32>, time: f32) -> vec2<f32> {
    return corner;
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
    @location(0) bounds: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) outline: vec4<f32>,
    @location(3) style: vec4<f32>,
    @location(4) transform: vec4<f32>,
    @location(5) transform_offset: vec4<f32>,
    @location(6) area: vec4<f32>,
) -> VertexOutput {
    // `area` is everything the surface can reach, in the node's own space:
    // the layers — which are free to sit outside the node that composes them —
    // widened by the outline, the softened edge and the bulge a smooth seam
    // adds. The host computes it, because only the host can see every layer.
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(area.x, area.y),
        vec2<f32>(area.z, area.y),
        vec2<f32>(area.x, area.w),
        vec2<f32>(area.z, area.w),
    );
    // Where a configuration's own shader gets to move a corner.
    //
    // The default returns it untouched, so an ordinary field costs nothing.
    // A vertex shader moves the *quad*, not the shape inside it: the fragment
    // stage still walks the field in the node's own space, so a displaced node
    // keeps its geometry and takes it somewhere else.
    let local = corners[vertex_index];
    // The displacement moves where the corner is *drawn*, not where the
    // fragment stage looks. `local` goes to the fragment unchanged, so the
    // field is still evaluated in the node's own space — displacing both would
    // move the quad and the shape inside it by the same amount and cancel out,
    // which is the shape this took before the GPU test caught it.
    let point = bounds.xy + morf_vertex_hook(local, bounds.zw, uniforms.viewport.z);
    let placed = vec2<f32>(
        transform.x * point.x + transform.z * point.y + transform_offset.x,
        transform.y * point.x + transform.w * point.y + transform_offset.y,
    );
    var output: VertexOutput;
    output.position = vec4<f32>(
        placed.x / uniforms.viewport.x * 2.0 - 1.0,
        1.0 - placed.y / uniforms.viewport.y * 2.0,
        0.0,
        1.0,
    );
    output.local = local;
    output.fill = fill;
    output.outline = outline;
    output.style = style;
    output.material = instance_index;
    return output;
}

fn rotate(point: vec2<f32>, degrees: f32) -> vec2<f32> {
    if degrees == 0.0 {
        return point;
    }
    let radians = -degrees * 0.017453292519943295;
    let c = cos(radians);
    let s = sin(radians);
    return vec2<f32>(point.x * c - point.y * s, point.x * s + point.y * c);
}

fn sd_circle(point: vec2<f32>, radius: f32) -> f32 {
    return length(point) - radius;
}

/// A box with a radius per corner, so an ordinary rect keeps its own shape when
/// a field absorbs it. `y` grows downwards here, as it does on the surface.
fn sd_capsule(point: vec2<f32>, half: vec2<f32>) -> f32 {
    // The stadium is a box whose corner radius is its own short half-extent.
    return sd_box_uniform(point, half, min(half.x, half.y));
}

fn sd_triangle(point: vec2<f32>, half: vec2<f32>) -> f32 {
    // An equilateral triangle inscribed in the layer box, pointing up.
    let k = sqrt(3.0);
    let r = min(half.x, half.y);
    var p = vec2<f32>(point.x / max(half.x, 0.0001), point.y / max(half.y, 0.0001)) * r;
    p.y = -p.y;
    p.x = abs(p.x) - r;
    p.y = p.y + r / k;
    if p.x + k * p.y > 0.0 {
        p = vec2<f32>(p.x - k * p.y, -k * p.x - p.y) / 2.0;
    }
    p.x = p.x - clamp(p.x, -2.0 * r, 0.0);
    return -length(p) * sign(p.y);
}

/// A regular hexagon inscribed in the layer box.
///
/// The classic form takes the apothem, whose circumradius is `2r/sqrt(3)` —
/// so passing the box's half-extent gave a hexagon 15% wider than the box it
/// was in, and the field's drawn area is computed from layer *rectangles*, so
/// the overhang was clipped off. Scaling to the apothem that inscribes puts
/// the widest points on the box edge, which is where every other family in the
/// vocabulary sits.
fn sd_hexagon(point: vec2<f32>, circumradius: f32) -> f32 {
    let radius = circumradius * 0.8660254037844386;
    let k = vec3<f32>(-0.8660254037844386, 0.5, 0.5773502691896258);
    var p = abs(point);
    p = p - 2.0 * min(dot(k.xy, p), 0.0) * k.xy;
    p = p - vec2<f32>(clamp(p.x, -k.z * radius, k.z * radius), radius);
    return length(p) * sign(p.y);
}

/// A star with a whole number of points.
fn sd_star_n(point: vec2<f32>, radius: f32, n: f32, inner: f32) -> f32 {
    let m = clamp(inner, 0.02, 0.98);
    // iq's regular polygon star, parameterised by the waist ratio.
    let an = 3.141592653589793 / n;
    let en = 3.141592653589793 / max(2.0 + m * (n - 2.0), 2.001);
    let racs = radius * vec2<f32>(cos(an), sin(an));
    let ecs = vec2<f32>(cos(en), sin(en));
    let bn = (atan2(abs(point.x), max(point.y, -1e20)) % (2.0 * an)) - an;
    var p = length(point) * vec2<f32>(cos(bn), abs(sin(bn)));
    p = p - racs;
    p = p + ecs * clamp(-dot(p, ecs), 0.0, racs.y / max(ecs.y, 0.0001));
    return length(p) * sign(p.x);
}

/// A star whose point count may be fractional.
///
/// A star is only defined for a whole number of points, so animating the count
/// through `floor` makes a new spike appear at full size between one frame and
/// the next. Blending the two neighbouring stars as *fields* instead grows the
/// new point out of the edge: at 5.5 points the surface is halfway between a
/// five-pointed and a six-pointed star, which is a shape neither of them
/// describes and exactly what the intermediate frames should show.
fn sd_star(point: vec2<f32>, radius: f32, points: f32, inner: f32) -> f32 {
    let n = max(3.0, points);
    let lower = floor(n);
    let fraction = n - lower;
    let a = sd_star_n(point, radius, lower, inner);
    if fraction <= 0.0001 {
        return a;
    }
    return mix(a, sd_star_n(point, radius, lower + 1.0, inner), fraction);
}

fn sd_ring(point: vec2<f32>, radius: f32, thickness: f32) -> f32 {
    let t = max(thickness, 0.0001);
    return abs(length(point) - radius + t * 0.5) - t * 0.5;
}

fn sd_pie(point: vec2<f32>, radius: f32, degrees: f32) -> f32 {
    // Centred on straight up, so a growing angle opens symmetrically.
    let half_angle = clamp(degrees, 0.0, 360.0) * 0.008726646259971648;
    let c = vec2<f32>(sin(half_angle), cos(half_angle));
    let p = vec2<f32>(abs(point.x), -point.y);
    let l = length(p) - radius;
    let m = length(p - c * clamp(dot(p, c), 0.0, radius));
    return max(l, m * sign(c.y * p.x - c.x * p.y));
}

fn sd_cross(point: vec2<f32>, half: vec2<f32>, thickness: f32) -> f32 {
    let t = max(thickness, 0.0001) * 0.5;
    let horizontal = sd_box_uniform(point, vec2<f32>(half.x, min(t, half.y)), 0.0);
    let vertical = sd_box_uniform(point, vec2<f32>(min(t, half.x), half.y), 0.0);
    return min(horizontal, vertical);
}

/// An ellipse filling the layer box.
///
/// The exact ellipse distance needs a root solve; this scales the point into
/// the unit circle and back by the shorter half-extent instead. The sign stays
/// exact — inside is exactly a normalised radius under one — and only the
/// magnitude is an underestimate, which widens the antialiasing ramp slightly
/// on a very eccentric ellipse. The sign is what the input-region rasteriser
/// reads, so a click and a pixel agree regardless.
fn sd_ellipse(point: vec2<f32>, half: vec2<f32>) -> f32 {
    let r = max(half, vec2<f32>(0.0001, 0.0001));
    return (length(point / r) - 1.0) * min(r.x, r.y);
}

/// Edges per bounding box, matching `glyph_layer::OUTLINE_SPAN`, which a test
/// asserts. The boxes are packed after the points, so where they start can be
/// worked out from the run rather than sent.
const OUTLINE_SPAN: u32 = 6u;

/// Distance to a closed outline, with the sign from its winding.
///
/// The same measurement the glyph fields use, done per pixel instead of once
/// into a texture — which is the whole point of a letter being a shape here
/// rather than a picture. A pixel walks the boxes and only opens the runs it
/// could be answered by, but it is still a walk per pixel: this is for the
/// letters a configuration composes with, not for a page of text.
fn sd_polygon(point: vec2<f32>, first: u32, stride: u32, loops: u32) -> f32 {
    if stride < 3u || loops == 0u {
        return 1.0e9;
    }
    var nearest = 1.0e9;
    var winding = 0;
    let spans = (stride + OUTLINE_SPAN - 1u) / OUTLINE_SPAN;
    let boxes = first + stride * loops;
    for (var contour = 0u; contour < loops; contour = contour + 1u) {
        // Each loop closes on itself rather than on the next one along, which
        // is what lets one layer hold a letter with holes in it: the counters
        // of `8` are wound against its body and cancel it, so they come out
        // hollow instead of being threaded onto it.
        let loop_start = first + contour * stride;
        for (var span = 0u; span < spans; span = span + 1u) {
            let corner = boxes + (contour * spans + span) * 2u;
            let low = outline[corner];
            let high = outline[corner + 1u];
            // A rightward ray can only meet a run that straddles the point's
            // height and reaches past it — the same half-open band the edge
            // test below uses, so a run on the boundary is dropped by both.
            let may_cross = point.y >= low.y && point.y < high.y && point.x < high.x;
            // And the box is a floor on how near the run's edges can come, so
            // a run further off than the nearest edge so far cannot win.
            let away = max(max(low - point, point - high), vec2<f32>(0.0, 0.0));
            if !may_cross && dot(away, away) >= nearest {
                continue;
            }
            for (var step = 0u; step < OUTLINE_SPAN; step = step + 1u) {
                let index = span * OUTLINE_SPAN + step;
                if index >= stride {
                    break;
                }
                let a = outline[loop_start + index];
                let b = outline[loop_start + (index + 1u) % stride];
                let edge = b - a;
                let to_point = point - a;
                let along = clamp(dot(to_point, edge) / max(dot(edge, edge), 1e-9), 0.0, 1.0);
                let offset = to_point - edge * along;
                nearest = min(nearest, dot(offset, offset));
                // A ray going right: an edge crossing the point's height flips
                // the count, and the total says inside from outside. Wound the
                // other way, a letter's counter cancels its body and comes out
                // hollow.
                let crosses = (a.y <= point.y) != (b.y <= point.y);
                if crosses {
                    let t = (point.y - a.y) / (b.y - a.y);
                    if a.x + t * edge.x > point.x {
                        winding = winding + select(-1, 1, b.y > a.y);
                    }
                }
            }
        }
    }
    return select(sqrt(nearest), -sqrt(nearest), winding != 0);
}

fn shape_distance(kind: u32, point: vec2<f32>, layer: Layer) -> f32 {
    let half = layer.rect.zw;
    let radius = min(half.x, half.y);
    switch kind {
        case 0u: { return sd_circle(point, radius); }
        case 1u: { return sd_box(point, half, layer.radii); }
        case 2u: { return sd_capsule(point, half); }
        case 3u: { return sd_triangle(point, half); }
        case 4u: { return sd_hexagon(point, radius); }
        case 5u: { return sd_star(point, radius, layer.params.y, layer.params.z); }
        case 6u: { return sd_ring(point, radius, layer.params.w); }
        case 7u: { return sd_pie(point, radius, layer.extra.x); }
        case 9u: { return sd_ellipse(point, half); }
        case 10u: {
            // Every contour is resampled to the same length when the outline
            // is built, so the stride is known here rather than sent. It must
            // match `morf_text::GLYPH_CONTOUR_POINTS`, which a test asserts.
            return sd_polygon(point, u32(layer.params.x), 96u, u32(layer.extra.w));
        }
        default: { return sd_cross(point, half, layer.params.w); }
    }
}

fn smooth_union(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

fn smooth_subtract(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 - 0.5 * (a + b) / k, 0.0, 1.0);
    return mix(a, -b, h) + k * h * (1.0 - h);
}

fn smooth_intersect(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 - 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) + k * h * (1.0 - h);
}

/// How much of the layer's own colour the combined surface takes on.
///
/// The same weight the distance operator uses, so colour follows the surface:
/// across a smooth seam the two fills cross-fade exactly where the two shapes
/// are bulging into one another. Only the operators that *add* surface bring a
/// colour with them — subtracting or intersecting removes area, it does not
/// paint.
fn combine_color_weight(
    accumulated: f32,
    layer_distance: f32,
    operation: u32,
    blend: f32,
) -> f32 {
    let k = max(blend, 0.0001);
    switch operation {
        // Union and xor both add surface, so both bring a colour with them.
        case 0u, 6u: { return select(0.0, 1.0, layer_distance < accumulated); }
        case 3u: {
            let h = clamp(0.5 + 0.5 * (layer_distance - accumulated) / k, 0.0, 1.0);
            return 1.0 - h;
        }
        default: { return 0.0; }
    }
}

fn combine(accumulated: f32, layer_distance: f32, operation: u32, blend: f32) -> f32 {
    // A smooth operator divides by its radius, so a zero radius is the hard
    // boolean rather than a division by zero.
    let k = max(blend, 0.0001);
    switch operation {
        case 0u: { return min(accumulated, layer_distance); }
        case 1u: { return max(accumulated, -layer_distance); }
        case 2u: { return max(accumulated, layer_distance); }
        case 3u: { return smooth_union(accumulated, layer_distance, k); }
        case 4u: { return smooth_subtract(accumulated, layer_distance, k); }
        case 5u: { return smooth_intersect(accumulated, layer_distance, k); }
        // Symmetric difference: outside the union, or inside the intersection,
        // whichever boundary is nearer.
        default: {
            return max(
                min(accumulated, layer_distance),
                -max(accumulated, layer_distance),
            );
        }
    }
}

/// The composed distance and fill at one point in the field's own space.
struct Composed {
    distance: f32,
    fill: vec4<f32>,
};

/// Walks the field's layers and resolves them into one surface.
///
/// Factored out because a shadow is the same composition sampled at an offset
/// point: one function, called twice, rather than a second copy of the loop
/// that could drift from this one.
fn compose(local: vec2<f32>, base: vec4<f32>, first: u32, count: u32) -> Composed {
    var out: Composed;
    out.distance = 1e20;
    out.fill = base;
    for (var index = 0u; index < count; index = index + 1u) {
        let layer = layers[first + index];
        let centred = rotate(local - layer.rect.xy, layer.extra.y);
        let start = shape_distance(u32(layer.kinds.x), centred, layer);
        // The morph is a straight interpolation of the two distance fields.
        // Where the fields disagree about which side of the edge a point is on,
        // the crossing moves continuously between them, so the outline can
        // split or merge without any correspondence between the two shapes.
        //
        // A layer that is not morphing — the common case, and every layer of a
        // composition that only moves — evaluates one shape rather than two.
        var value = start;
        if layer.kinds.z > 0.0 && u32(layer.kinds.y) != u32(layer.kinds.x) {
            value = mix(
                start,
                shape_distance(u32(layer.kinds.y), centred, layer),
                layer.kinds.z,
            );
        }
        if index == 0u {
            out.distance = value;
            out.fill = layer.color;
        } else {
            let weight = combine_color_weight(
                out.distance,
                value,
                u32(layer.kinds.w),
                layer.extra.z,
            );
            out.fill = mix(out.fill, layer.color, weight);
            out.distance = combine(out.distance, value, u32(layer.kinds.w), layer.extra.z);
        }
    }
    return out;
}

/// Where a configuration's own shader gets to decide the colour.
///
/// The default hands back what the field already resolved, so the base pipeline
/// costs nothing. When a node carries a shader, this body is replaced at
/// pipeline creation with a call into the compiled `morf_shader_main` — a
/// distinct pipeline per shader, because WGSL has no way to swap a function at
/// run time and a branch on a uniform would pay for a shader on every node that
/// does not have one.
///
/// The shape still comes from the field, so clipping, damage, hit testing and
/// the input region are untouched by anything a shader does.
fn morf_shader_hook(
    uv: vec2<f32>,
    local: vec2<f32>,
    coverage: f32,
    base: vec4<f32>,
) -> vec4<f32> {
    return base;
}

/// Who decides how much of this pixel the node covers.
///
/// A material shader colours what the field already shaped, so the default
/// hands back the field's own coverage. A surface shader decides for itself,
/// and this body is replaced with one that returns the shader's alpha —
/// geometry and shader stop composing there, which is inherent to the mode
/// rather than a gap in it.
fn morf_coverage_hook(shader_alpha: f32, filled: f32) -> f32 {
    return filled;
}

/// The fill a gradient paints at this point, or the flat colour if there is none.
fn gradient_fill(material: Material, local: vec2<f32>, flat_color: vec4<f32>) -> vec4<f32> {
    let kind = material.gradient_data.x;
    if kind == 0.0 {
        return flat_color;
    }
    // Normalised across the material's own rectangle, so a gradient reads the
    // same whatever the node's size — which is what a configuration means by
    // giving its endpoints as fractions.
    let point = local - material.shape.xy;
    let normalized = point / max(material.shape.zw, vec2<f32>(0.000001));
    if kind == 1.0 {
        let direction = material.gradient_points.zw - material.gradient_points.xy;
        let amount = dot(normalized - material.gradient_points.xy, direction)
            / max(dot(direction, direction), 0.000001);
        return mix(
            material.gradient_start_color,
            material.gradient_end_color,
            clamp(amount, 0.0, 1.0),
        );
    }
    if kind == 2.0 {
        let amount = distance(normalized, material.gradient_data.yz)
            / max(material.gradient_data.w, 0.000001);
        return mix(
            material.gradient_start_color,
            material.gradient_end_color,
            clamp(amount, 0.0, 1.0),
        );
    }
    let delta = normalized - material.gradient_data.yz;
    let amount = fract((atan2(delta.y, delta.x) - material.effects.w) / 6.28318530718);
    return mix(material.gradient_start_color, material.gradient_end_color, amount);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let first = u32(input.style.z);
    let count = u32(input.style.w);
    let material = materials[input.material];
    let surface = compose(input.local, input.fill, first, count);
    let distance = surface.distance;

    // The derivative gives one pixel of coverage whatever the surface scale,
    // and `softness` widens it deliberately on top of that.
    let edge = max(fwidth(distance), 0.0001) * 0.5;
    let softness = max(input.style.y, edge);
    // An unantialiased border is a hard step: the same crossing, with no ramp.
    let antialiased = material.border.y > 0.5;
    let ramp = select(0.0001, softness, antialiased);

    // One outline, placed by its alignment. Inside is what a rectangle border
    // has always meant; centred is what a field stroke has always meant. They
    // were two mechanisms in two shaders for the same band of pixels.
    let width = max(input.style.x, 0.0);
    let alignment = material.border.x;
    let outset = select(select(0.0, width, alignment >= 1.5), width * 0.5, alignment == 1.0);
    let inset = width - outset;

    let coverage = smoothstep(ramp, -ramp, distance - outset);
    let filled = smoothstep(ramp, -ramp, distance + inset);

    var fill_color = gradient_fill(material, input.local, surface.fill);
    // Normalised across the node's own rectangle, which is what a shader means
    // by `uv` and what makes one read the same at any size.
    let shader_uv = (input.local - material.shape.xy) / max(material.shape.zw, vec2<f32>(0.000001));
    fill_color = morf_shader_hook(shader_uv, input.local, coverage, fill_color);
    let fill_alpha = fill_color.a * morf_coverage_hook(fill_color.a, filled);
    let outline_alpha = input.outline.a * max(coverage - filled, 0.0);
    let shape = vec4<f32>(
        fill_color.rgb * fill_alpha + input.outline.rgb * outline_alpha,
        fill_alpha + outline_alpha,
    );

    var result = shape;
    if material.shadow_color.a > 0.0 {
        // A shadow is this same composition, moved and dilated. Dilating a
        // signed distance field is subtraction — no need to grow every layer's
        // rectangle and hope the corners follow, which is what the rounded-box
        // shadow used to do.
        let spread = material.effects.z;
        let shadow_softness = max(material.effects.y, edge);
        let inner = material.shadow.z > 0.5;
        let offset_point = input.local - material.shadow.xy;
        let shadow_distance =
            compose(offset_point, input.fill, first, count).distance - spread;
        if inner {
            // Inside the surface, darkened where the offset shape is *not*.
            let amount = material.shadow_color.a
                * coverage
                * smoothstep(-shadow_softness, shadow_softness, shadow_distance);
            result = vec4<f32>(
                mix(result.rgb, material.shadow_color.rgb * result.a, amount),
                result.a,
            );
        } else {
            let alpha = material.shadow_color.a
                * smoothstep(shadow_softness, -shadow_softness, shadow_distance);
            let layer = vec4<f32>(material.shadow_color.rgb * alpha, alpha);
            result = shape + layer * (1.0 - shape.a);
        }
    }

    return vec4<f32>(
        mix(result.rgb, material.color_overlay.rgb * result.a, material.color_overlay.a),
        result.a,
    );
}
