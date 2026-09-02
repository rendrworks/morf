// A letter as a shape in a distance-field composition.
//
// Every other shape a field can hold is a family with parameters — a circle has
// a radius, a star has a point count. A glyph is neither: it is one particular
// outline, so it reaches the composition as points and the layer says where its
// own run of them begins.

use morf_region::Shape;

use crate::commands::SdfLayer;

/// A layer's shape parameters, and how many contours its outline has.
///
/// The outline goes in the two slots nothing else uses — the padding at the
/// front of `params` and the tail of `extra` — because every other slot already
/// means something to some shape. Putting the run there instead cost a star its
/// point count and its waist, and `morph_to = "star"` came out as a
/// ninety-six-pointed scribble.
///
/// The points are placed into the layer's own box — centred, and scaled to fit
/// without distorting the letter — because a field layer is positioned by its
/// rectangle and a glyph arrives in font coordinates that know nothing about it.
pub(crate) fn polygon_params(
    layer: &SdfLayer,
    scale: f64,
    outlines: &mut Vec<[f32; 2]>,
    text: &mut morf_text::TextSystem,
) -> ([f32; 4], f32) {
    let plain = [
        0.0,
        layer.points,
        layer.inner_radius,
        (f64::from(layer.thickness) * scale) as f32,
    ];
    // Either end being an outline means the points are needed: a star morphing
    // into a letter reads the letter's points just as the letter morphing into
    // a star does.
    if layer.shape != Shape::Polygon && layer.morph_to != Shape::Polygon {
        return (plain, 0.0);
    }
    let Some(glyph) = layer.glyph else {
        return (plain, 0.0);
    };
    let points = text.glyph_outline(glyph, layer.glyph_morph_to, layer.morph);
    if points.len() < 3 {
        return (plain, 0.0);
    }

    // Font coordinates count upwards from a baseline; a field's box counts
    // downwards from its top-left. One scale for both axes, so the letter keeps
    // its proportions inside whatever rectangle it was given.
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for point in &points {
        min_x = min_x.min(point.0);
        max_x = max_x.max(point.0);
        min_y = min_y.min(point.1);
        max_y = max_y.max(point.1);
    }
    let half_width = ((layer.bounds.width / 2.0) * scale) as f32;
    let half_height = ((layer.bounds.height / 2.0) * scale) as f32;
    let span_x = (max_x - min_x).max(f32::EPSILON);
    let span_y = (max_y - min_y).max(f32::EPSILON);
    let fit = (half_width * 2.0 / span_x).min(half_height * 2.0 / span_y);
    let centre = ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);

    let first = outlines.len() as f32;
    for point in &points {
        outlines.push([
            (point.0 - centre.0) * fit,
            // Flipped: the shader walks a field whose y grows downwards.
            -(point.1 - centre.1) * fit,
        ]);
    }
    // Where the run starts, and how many closed loops are in it. Every loop is
    // `GLYPH_CONTOUR_POINTS` long, which the shader knows.
    let loops = (points.len() / morf_text::GLYPH_CONTOUR_POINTS) as f32;
    ([first, plain[1], plain[2], plain[3]], loops)
}
