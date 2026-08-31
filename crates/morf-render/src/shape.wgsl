// The distance functions every shader shares.
//
// Prepended to each shader source at pipeline creation, because WGSL has no
// include and the alternative is what this file replaced: three copies of the
// same rounded box, which had already drifted into rendering the same numbers
// as three different shapes.

/// Signed distance to a rounded box, from a point relative to its centre.
///
/// The radius is clamped to the box's own half-extent, so asking for a corner
/// larger than the box can hold gives the largest one it can — a capsule, and
/// then a circle — rather than a shape that inverts. That clamp is what makes
/// the `radius = 9999` pill idiom mean the same thing everywhere.
fn sd_box(point: vec2<f32>, half: vec2<f32>, radii: vec4<f32>) -> f32 {
    var r = select(
        select(radii.x, radii.w, point.y >= 0.0),
        select(radii.y, radii.z, point.y >= 0.0),
        point.x >= 0.0,
    );
    r = min(max(r, 0.0), min(half.x, half.y));
    let q = abs(point) - half + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_box_uniform(point: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    return sd_box(point, half, vec4<f32>(radius, radius, radius, radius));
}

/// The same box, from a point measured from its top-left corner.
fn rounded_distance(point: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half = size * 0.5;
    return sd_box(point - half, half, radii);
}
