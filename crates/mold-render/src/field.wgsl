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

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> layers: array<Layer>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) outline: vec4<f32>,
    // [stroke width, softness, first layer, layer count]
    @location(3) style: vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) bounds: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) outline: vec4<f32>,
    @location(3) style: vec4<f32>,
    @location(4) transform: vec4<f32>,
    @location(5) transform_offset: vec4<f32>,
) -> VertexOutput {
    // The quad is expanded by however far the surface may reach outside the
    // node: the outline, the softened edge, and the bulge a smooth seam adds.
    // The host computes it, because only the host can see every layer's blend.
    let spread = transform_offset.z;
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-spread, -spread),
        vec2<f32>(bounds.z + spread, -spread),
        vec2<f32>(-spread, bounds.w + spread),
        vec2<f32>(bounds.z + spread, bounds.w + spread),
    );
    let local = corners[vertex_index];
    let point = bounds.xy + local;
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
fn sd_box(point: vec2<f32>, half: vec2<f32>, radii: vec4<f32>) -> f32 {
    var r = select(
        select(radii.x, radii.w, point.y >= 0.0),
        select(radii.y, radii.z, point.y >= 0.0),
        point.x >= 0.0,
    );
    r = min(r, min(half.x, half.y));
    let q = abs(point) - half + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_box_uniform(point: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    return sd_box(point, half, vec4<f32>(radius, radius, radius, radius));
}

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

fn sd_hexagon(point: vec2<f32>, radius: f32) -> f32 {
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
        case 0u: { return select(0.0, 1.0, layer_distance < accumulated); }
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
        default: { return smooth_intersect(accumulated, layer_distance, k); }
    }
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let first = u32(input.style.z);
    let count = u32(input.style.w);
    var distance = 1e20;
    var fill = input.fill;
    for (var index = 0u; index < count; index = index + 1u) {
        let layer = layers[first + index];
        let centred = rotate(input.local - layer.rect.xy, layer.extra.y);
        let start = shape_distance(u32(layer.kinds.x), centred, layer);
        let finish = shape_distance(u32(layer.kinds.y), centred, layer);
        // The morph is a straight interpolation of the two distance fields.
        // Where the fields disagree about which side of the edge a point is on,
        // the crossing moves continuously between them, so the outline can
        // split or merge without any correspondence between the two shapes.
        let value = mix(start, finish, layer.kinds.z);
        if index == 0u {
            distance = value;
            fill = layer.color;
        } else {
            let weight = combine_color_weight(
                distance,
                value,
                u32(layer.kinds.w),
                layer.extra.z,
            );
            fill = mix(fill, layer.color, weight);
            distance = combine(distance, value, u32(layer.kinds.w), layer.extra.z);
        }
    }
    // The derivative gives one pixel of coverage whatever the surface scale,
    // and `softness` widens it deliberately on top of that.
    let edge = max(fwidth(distance), 0.0001) * 0.5;
    let softness = max(input.style.y, edge);
    let coverage = smoothstep(softness, -softness, distance);
    var color = fill;
    color.a = color.a * coverage;
    let stroke = input.style.x;
    if stroke > 0.0 && input.outline.a > 0.0 {
        // Centred on the crossing, so growing the width does not move the edge.
        let outline_coverage = smoothstep(softness, -softness, abs(distance) - stroke * 0.5);
        let outline_alpha = input.outline.a * outline_coverage;
        color = vec4<f32>(
            mix(color.rgb, input.outline.rgb, outline_alpha),
            color.a + outline_alpha * (1.0 - color.a),
        );
    }
    return vec4<f32>(color.rgb * color.a, color.a);
}
