// Glyphs as distance fields, measured from the outline rather than from a
// picture of it.
//
// A coverage bitmap is a glyph at one size with one subpixel offset, worth
// nothing at any other; a distance field is the *shape*, so one entry serves
// every size and an outline or a heavier weight is a second threshold rather
// than a re-render.
//
// The field is computed from the font's own Bézier contours. The obvious way
// to build one is to rasterize the glyph, threshold that bitmap and run a
// distance transform over it — and that is what this did. It is wrong in a way
// that no amount of supersampling fixes: thresholding throws the shape away and
// keeps a grid of on-or-off pixels, so every distance afterwards is measured to
// a staircase rather than to a curve. What comes back is an approximation of an
// approximation, and it shows up as chewed edges at large sizes and as
// disintegrating letterforms when two fields are interpolated.
//
// The outline is already in the font, exactly. Measuring straight to it makes
// the distances true, lets the spread be as wide as we like — the transform's
// cost used to grow with it — and removes the rasterizer from the path
// entirely.

use std::rc::Rc;

use cosmic_text::Command;

use crate::glyph_msdf::{colour_contour, distances};

/// The size a field is measured at when nothing narrows it down.
///
/// A field records a shape, not a size, so one entry can serve any size — but
/// only within reason. Reading a sixty-four pixel field into eleven pixels of
/// text means resolving six texels into every screen pixel with nothing but a
/// bilinear tap, and the letters come back scarred. So the reference is chosen
/// per size instead; see [`field_reference_for`].
pub const FIELD_REFERENCE_PX: f32 = 64.0;

/// The reference size a glyph drawn at `size` is measured at.
///
/// Powers of two, so a size that animates settles on a handful of fields rather
/// than one per frame, and so a field is never read at worse than half its own
/// resolution — which is the point: minification is what tore small text, not
/// the field. Bounded below because a field smaller than this stops describing
/// a letter, and above because past it the extra detail is beyond the screen.
pub fn field_reference_for(size: f32) -> f32 {
    let wanted = size.max(1.0).log2().ceil().exp2();
    wanted.clamp(16.0, 128.0)
}

/// How far outside the glyph a field of this reference size is measured.
///
/// Capped, and the cap is what decides how good the edges look. A field is a
/// byte: two hundred and fifty-five levels spread over twice this distance. The
/// only ones that matter are those inside the pixel the edge falls in, so the
/// wider the spread the fewer levels there are to draw that pixel with — at a
/// spread proportional to the reference, a sixty-four pixel letter had five
/// greys across its edge and a three hundred pixel one had barely one, which is
/// what stepped curves are made of.
///
/// Eight reference pixels is far more than an edge, a weight or an outline
/// needs, and it leaves sixteen greys to draw with at the reference size rather
/// than five. Nothing needs the wider range any more: a letter composed into a
/// field is an outline walked exactly, not a texture sampled, so its reach is
/// not bounded by this at all.
pub fn field_spread_for(reference: f32) -> f32 {
    (reference * 0.375).min(8.0).round()
}

/// How much of a field one logical pixel covers, for a glyph drawn at `size`.
///
/// The one place the reference and the spread are turned into the units the
/// shader thresholds in. Everything that has to speak in field units — the
/// width of an edge, a weight, an outline — goes through here, because the
/// relation between the two stopped being a fixed ratio when the spread gained
/// a cap, and two copies of it would have quietly disagreed.
pub fn field_units_per_logical_px(size: f32) -> f32 {
    let reference = field_reference_for(size);
    let spread = field_spread_for(reference);
    (reference / size.max(1.0)) / (spread * 2.0)
}

/// The spread of a field measured at the default reference size.
pub const FIELD_SPREAD_PX: u32 = 24;

/// How finely a curve is broken into straight pieces before it is measured.
///
/// In reference pixels of allowed deviation. The distance to a chord this close
/// to the curve is wrong by less than the eighth of a pixel the stored byte can
/// express, so nothing is gained by going finer.
const FLATTEN_TOLERANCE: f32 = 0.05;

/// One straight piece of an outline.
pub(crate) struct Segment {
    pub(crate) x0: f32,
    pub(crate) y0: f32,
    pub(crate) x1: f32,
    pub(crate) y1: f32,
}

impl Segment {
    /// Distance from a point to this piece, squared.
    pub(crate) fn distance_squared(&self, x: f32, y: f32) -> f32 {
        let (dx, dy) = (self.x1 - self.x0, self.y1 - self.y0);
        let length = dx * dx + dy * dy;
        let t = if length <= f32::EPSILON {
            0.0
        } else {
            (((x - self.x0) * dx + (y - self.y0) * dy) / length).clamp(0.0, 1.0)
        };
        let (nx, ny) = (self.x0 + t * dx - x, self.y0 + t * dy - y);
        nx * nx + ny * ny
    }

    /// Whether a ray going right from the point crosses this piece, and which
    /// way round. Summed over the outline this is the winding number, which is
    /// what says inside from outside — including the counters of `o` and `8`,
    /// which are wound the other way and so cancel.
    pub(crate) fn winding(&self, x: f32, y: f32) -> i32 {
        if (self.y0 <= y) == (self.y1 <= y) {
            return 0;
        }
        let t = (y - self.y0) / (self.y1 - self.y0);
        if self.x0 + t * (self.x1 - self.x0) <= x {
            return 0;
        }
        if self.y1 > self.y0 { 1 } else { -1 }
    }
}

/// Breaks the outline into straight pieces, closing every subpath.
///
/// A distance is measured to the whole boundary, so an unclosed subpath would
/// leave a gap the winding count could escape through and the letter would come
/// back inside-out. Fonts close their contours, but a `MoveTo` without a
/// `Close` before it is legal and has to be treated as one.
pub(crate) fn flatten(commands: &[Command]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut start = (0.0_f32, 0.0_f32);
    let mut at = start;

    let line = |from: (f32, f32), to: (f32, f32), into: &mut Vec<Segment>| {
        if from != to {
            into.push(Segment {
                x0: from.0,
                y0: from.1,
                x1: to.0,
                y1: to.1,
            });
        }
    };

    for command in commands {
        match command {
            Command::MoveTo(point) => {
                line(at, start, &mut segments);
                start = (point.x, point.y);
                at = start;
            }
            Command::LineTo(point) => {
                let to = (point.x, point.y);
                line(at, to, &mut segments);
                at = to;
            }
            Command::QuadTo(control, point) => {
                let control = (control.x, control.y);
                let to = (point.x, point.y);
                for (from, next) in quadratic_steps(at, control, to) {
                    line(from, next, &mut segments);
                }
                at = to;
            }
            Command::CurveTo(first, second, point) => {
                let first = (first.x, first.y);
                let second = (second.x, second.y);
                let to = (point.x, point.y);
                for (from, next) in cubic_steps(at, first, second, to) {
                    line(from, next, &mut segments);
                }
                at = to;
            }
            Command::Close => {
                line(at, start, &mut segments);
                at = start;
            }
        }
    }
    line(at, start, &mut segments);
    segments
}

/// How many pieces a curve needs, from how far its controls stray.
///
/// The control polygon is never shorter than the curve, so a step count taken
/// from its length is never too few — which is the direction to be wrong in.
fn steps_for(deviation: f32) -> usize {
    ((deviation / FLATTEN_TOLERANCE).sqrt().ceil() as usize).clamp(1, 64)
}

fn quadratic_steps(
    from: (f32, f32),
    control: (f32, f32),
    to: (f32, f32),
) -> Vec<((f32, f32), (f32, f32))> {
    let deviation =
        (control.0 - (from.0 + to.0) * 0.5).abs() + (control.1 - (from.1 + to.1) * 0.5).abs();
    let steps = steps_for(deviation);
    let mut pieces = Vec::with_capacity(steps);
    let mut previous = from;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let inverse = 1.0 - t;
        let point = (
            inverse * inverse * from.0 + 2.0 * inverse * t * control.0 + t * t * to.0,
            inverse * inverse * from.1 + 2.0 * inverse * t * control.1 + t * t * to.1,
        );
        pieces.push((previous, point));
        previous = point;
    }
    pieces
}

fn cubic_steps(
    from: (f32, f32),
    first: (f32, f32),
    second: (f32, f32),
    to: (f32, f32),
) -> Vec<((f32, f32), (f32, f32))> {
    let deviation = (first.0 - from.0).abs()
        + (first.1 - from.1).abs()
        + (second.0 - to.0).abs()
        + (second.1 - to.1).abs();
    let steps = steps_for(deviation);
    let mut pieces = Vec::with_capacity(steps);
    let mut previous = from;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let inverse = 1.0 - t;
        let (a, b) = (inverse * inverse * inverse, 3.0 * inverse * inverse * t);
        let (c, d) = (3.0 * inverse * t * t, t * t * t);
        let point = (
            a * from.0 + b * first.0 + c * second.0 + d * to.0,
            a * from.1 + b * first.1 + c * second.1 + d * to.1,
        );
        pieces.push((previous, point));
        previous = point;
    }
    pieces
}

/// Measures a glyph's outline into a distance field.
///
/// The outline arrives in font coordinates: the origin is the pen on the
/// baseline and y counts upwards, which is why the rows below are walked from
/// the top down. Returns `None` for an outline with no ink — a space has no
/// boundary to measure a distance from.
pub(crate) fn glyph_field(commands: &[Command], spread: f32) -> Option<FieldImage> {
    glyph_field_in(commands, outline_box(commands, spread)?, spread)
}

/// The box a glyph's field is measured over: its ink, with the spread around
/// it. In reference pixels relative to the pen, y counting up.
pub(crate) fn outline_box(commands: &[Command], spread: f32) -> Option<FieldBox> {
    segment_box(&flatten(commands), spread)
}

/// The box a set of straight pieces needs, with the spread around them.
pub(crate) fn segment_box(segments: &[Segment], spread: f32) -> Option<FieldBox> {
    if segments.is_empty() {
        return None;
    }
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for segment in segments {
        min_x = min_x.min(segment.x0).min(segment.x1);
        max_x = max_x.max(segment.x0).max(segment.x1);
        min_y = min_y.min(segment.y0).min(segment.y1);
        max_y = max_y.max(segment.y0).max(segment.y1);
    }
    // Not rounded to whole pixels. The box's edges are where the outline
    // actually is, because this origin is what the quad is positioned from: a
    // box snapped to the reference grid moves the letter by up to a whole
    // reference pixel, which at a small drawn size is most of a pixel on
    // screen and reads as letters standing unevenly apart.
    Some(FieldBox {
        left: min_x - spread,
        top: max_y + spread,
        right: max_x + spread,
        bottom: min_y - spread,
    })
}

/// A rectangle in reference pixels, relative to the pen, with y counting up.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FieldBox {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) right: f32,
    pub(crate) bottom: f32,
}

impl FieldBox {
    /// The smallest box holding both.
    ///
    /// Two glyphs being interpolated have to be measured over the *same* box.
    /// Their own boxes differ in width, in height and in where the ink sits
    /// inside them, and the shader reads both through one set of texture
    /// coordinates — so separate boxes are stretched onto one another, which
    /// distorts the shapes and, worse, invalidates the distances they store,
    /// since those are in units of the box they were measured in.
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }
}

/// Measures an outline over a box chosen by the caller.
pub(crate) fn glyph_field_in(
    commands: &[Command],
    area: FieldBox,
    spread: f32,
) -> Option<FieldImage> {
    field_from_segments(&flatten(commands), area, spread)
}

/// Measures an outline already broken into straight pieces.
///
/// The pieces need not have come from a font: an outline interpolated between
/// two letters is measured exactly the same way, which is what lets a morph be
/// a real shape rather than an average of two fields.
pub(crate) fn field_from_segments(
    segments: &[Segment],
    area: FieldBox,
    spread: f32,
) -> Option<FieldImage> {
    if segments.is_empty() {
        return None;
    }

    let left = area.left;
    let top = area.top;
    // The grid is whole texels even though the box is not: the extra fraction
    // of a texel falls outside the spread, where the field is saturated anyway.
    let width = (area.right - area.left).ceil().max(1.0) as u32;
    let height = (area.top - area.bottom).ceil().max(1.0) as u32;

    // Each contour's edges are shared out between three channels, so a corner
    // can break one of them; see `glyph_msdf`.
    let coloured: Vec<_> = split_contours(segments)
        .into_iter()
        .flat_map(colour_contour)
        .collect();

    let fold = |distance: f32| {
        // Negative inside, positive outside, then folded into the byte the
        // shader thresholds: zero is deep inside a stroke and one is further
        // out than the spread reaches.
        (((distance + spread) / (spread * 2.0)).clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        // Sampled at the centre of the texel, and downwards: the field is read
        // as an image, where the first row is the top, while the outline it is
        // measured from counts y upwards from the baseline.
        let y = top - row as f32 - 0.5;
        for column in 0..width {
            let x = left + column as f32 + 0.5;
            let (channels, overall) = distances(&coloured, x, y);
            data.push(fold(channels[0]));
            data.push(fold(channels[1]));
            data.push(fold(channels[2]));
            // The plain distance alongside the three, for everything that wants
            // a distance rather than a contour — softness, and a fallback where
            // the median has nothing to reconstruct.
            data.push(fold(overall));
        }
    }

    Some(FieldImage {
        left,
        top,
        width,
        height,
        data: Rc::new(data),
    })
}

/// One glyph's field, and where it sits relative to the pen.
///
/// `left` and `top` are the reference-size placement of the *padded* box, so
/// scaling them by the size being drawn gives the quad directly.
pub(crate) struct FieldImage {
    /// Where the box sits relative to the pen, in reference pixels.
    ///
    /// Fractional, and it matters: this is what the quad is placed from, and
    /// rounding it here is rounding the letter's position on screen.
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) data: Rc<Vec<u8>>,
}

/// Splits flattened pieces back into the closed loops they came from.
///
/// `flatten` closes every subpath, so a loop ends wherever the next piece does
/// not continue from the last one. The colouring is per contour: a corner is a
/// property of two edges meeting, and the last piece of one loop does not meet
/// the first piece of the next.
fn split_contours(segments: &[Segment]) -> Vec<Vec<Segment>> {
    let mut loops = Vec::new();
    let mut current: Vec<Segment> = Vec::new();
    for piece in segments {
        if let Some(last) = current.last() {
            let apart = (last.x1 - piece.x0).abs() + (last.y1 - piece.y0).abs();
            if apart > 0.01 {
                loops.push(std::mem::take(&mut current));
            }
        }
        current.push(Segment {
            x0: piece.x0,
            y0: piece.y0,
            x1: piece.x1,
            y1: piece.y1,
        });
    }
    if !current.is_empty() {
        loops.push(current);
    }
    loops
}
