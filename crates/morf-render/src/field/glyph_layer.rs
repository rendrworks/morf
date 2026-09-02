// A letter as a shape in a distance-field composition.
//
// Every other shape a field can hold is a family with parameters — a circle has
// a radius, a star has a point count. A glyph is neither: it is one particular
// outline, so it reaches the composition as points and the layer says where its
// own run of them begins.

use morf_region::Shape;

use crate::commands::SdfLayer;

/// How many edges share one bounding box.
///
/// A fragment measures itself against every edge of an outline, so a letter
/// costs a hundred-odd steps per pixel and twice that under a shadow. Boxing
/// the edges in runs lets most of those runs be skipped: one that is further
/// away than the nearest edge found so far, and out of the band a rightward
/// ray could cross, cannot change either the distance or the winding. Six cuts
/// a contour into sixteen runs and costs a third of its points again in boxes;
/// against four, eight, twelve and sixteen it opened the fewest edges for the
/// box tests it added — a little over a third of the letter, where measuring
/// everything is the whole of it.
pub(crate) const OUTLINE_SPAN: usize = 6;

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
    drawings: &mut morf_svg::SvgOutlines,
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
    // A letter or a drawing: two ways of writing down the same thing, and by
    // here the difference is only which of the two a layer happened to name.
    let points = match (layer.glyph, layer.svg_source.as_deref()) {
        // A letter walking onto a drawing, which is the same thing said the
        // other way round.
        (Some(glyph), Some(source)) if layer.morph > 0.0 => {
            let family = layer.font_family.as_deref().unwrap_or("sans-serif");
            morf_svg::SvgOutlines::walk_between(
                text.glyph_contours(glyph, family),
                drawings.contours_of(source),
                layer.morph,
            )
        }
        (Some(glyph), _) => {
            let family = layer.font_family.as_deref().unwrap_or("sans-serif");
            let family_to = layer.font_family_morph_to.as_deref().unwrap_or(family);
            text.glyph_outline(glyph, layer.glyph_morph_to, layer.morph, family, family_to)
        }
        (None, Some(source)) => match layer.glyph_morph_to {
            // A drawing walking onto a letter. The two are cached apart, so the
            // loops are fetched from both sides and paired here — which is the
            // whole of the difference, because from `pair_up` onwards neither
            // side can tell what the other one was written in.
            Some(letter) if layer.morph > 0.0 => {
                let family = layer.font_family.as_deref().unwrap_or("sans-serif");
                morf_svg::SvgOutlines::walk_between(
                    drawings.contours_of(source),
                    text.glyph_contours(letter, family),
                    layer.morph,
                )
            }
            _ => drawings.outline(source, layer.svg_source_morph_to.as_deref(), layer.morph),
        },
        (None, None) => return (plain, 0.0),
    };
    if points.len() < 3 {
        return (plain, 0.0);
    }

    // Font coordinates count upwards from a baseline; a field's box counts
    // downwards from its top-left. One scale for both axes, so the shape keeps
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

    let first = outlines.len();
    for point in &points {
        outlines.push([
            (point.0 - centre.0) * fit,
            // Flipped: the shader walks a field whose y grows downwards.
            -(point.1 - centre.1) * fit,
        ]);
    }
    // Where the run starts, and how many closed loops are in it. Every loop is
    // `GLYPH_CONTOUR_POINTS` long, which the shader knows.
    let loops = points.len() / morf_text::GLYPH_CONTOUR_POINTS;
    push_span_boxes(outlines, first, loops);
    ([first as f32, plain[1], plain[2], plain[3]], loops as f32)
}

/// Boxes each run of edges, after the points the runs are cut from.
///
/// They go behind the points rather than in a buffer of their own because the
/// shader can find them without being told where: the contours are a fixed
/// length, so the run of boxes starts exactly where the points end.
///
/// A box holds the run's edges, which means one point more than the run's own
/// — the last edge of a run ends on the first point of the next, and the last
/// run of a contour closes on the contour's first point.
///
/// And it is loosened by a hair afterwards. A box that fits its run exactly is
/// only exact in arithmetic: the shader compares it against numbers it works
/// out edge by edge, so a crossing that lands on the boundary can be ruled out
/// by a rounding error in the last bit, and a letter turns inside out along a
/// hairline. The slack is far under a pixel and far over the error.
fn push_span_boxes(outlines: &mut Vec<[f32; 2]>, first: usize, loops: usize) {
    const SLACK: f32 = 1.0 / 1024.0;
    let stride = morf_text::GLYPH_CONTOUR_POINTS;
    for contour in 0..loops {
        let start = first + contour * stride;
        for span in (0..stride).step_by(OUTLINE_SPAN) {
            let mut low = [f32::MAX, f32::MAX];
            let mut high = [f32::MIN, f32::MIN];
            for step in 0..=OUTLINE_SPAN {
                let point = outlines[start + (span + step) % stride];
                low = [low[0].min(point[0]), low[1].min(point[1])];
                high = [high[0].max(point[0]), high[1].max(point[1])];
            }
            let slack = SLACK * (1.0 + (high[0] - low[0]).max(high[1] - low[1]));
            outlines.push([low[0] - slack, low[1] - slack]);
            outlines.push([high[0] + slack, high[1] + slack]);
        }
    }
}
