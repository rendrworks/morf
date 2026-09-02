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

/// The size the outline is measured at, and the units every field is in.
///
/// The field records a shape, not a size: text is drawn by scaling the quad,
/// and nothing is measured again. This only sets how finely the curve is
/// sampled and how much atlas one letter costs.
pub const FIELD_REFERENCE_PX: f32 = 64.0;

/// How far outside the glyph the field is measured, in reference pixels.
///
/// This is the room an outline has to live in, the distance the edge can be
/// moved to thicken a letter, and — the reason it is as wide as it is — the
/// range over which two glyphs can be interpolated. Everything beyond it reads
/// as "far outside", and two letters whose strokes are further apart than this
/// have nothing to interpolate: the stroke cannot travel in, it can only appear.
/// At eight pixels, which is what the distance transform could afford, most
/// pairs fell apart in the middle.
pub const FIELD_SPREAD_PX: u32 = 24;

/// How finely a curve is broken into straight pieces before it is measured.
///
/// In reference pixels of allowed deviation. The distance to a chord this close
/// to the curve is wrong by less than the eighth of a pixel the stored byte can
/// express, so nothing is gained by going finer.
const FLATTEN_TOLERANCE: f32 = 0.05;

/// One straight piece of an outline.
struct Segment {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Segment {
    /// Distance from a point to this piece, squared.
    fn distance_squared(&self, x: f32, y: f32) -> f32 {
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
    fn winding(&self, x: f32, y: f32) -> i32 {
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
fn flatten(commands: &[Command]) -> Vec<Segment> {
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
pub(crate) fn glyph_field(commands: &[Command]) -> Option<FieldImage> {
    glyph_field_in(commands, outline_box(commands)?)
}

/// The box a glyph's field is measured over: its ink, with the spread around
/// it. In reference pixels relative to the pen, y counting up.
pub(crate) fn outline_box(commands: &[Command]) -> Option<FieldBox> {
    let segments = flatten(commands);
    if segments.is_empty() {
        return None;
    }
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for segment in &segments {
        min_x = min_x.min(segment.x0).min(segment.x1);
        max_x = max_x.max(segment.x0).max(segment.x1);
        min_y = min_y.min(segment.y0).min(segment.y1);
        max_y = max_y.max(segment.y0).max(segment.y1);
    }
    let spread = FIELD_SPREAD_PX as f32;
    Some(FieldBox {
        left: (min_x - spread).floor(),
        top: (max_y + spread).ceil(),
        right: (max_x + spread).ceil(),
        bottom: (min_y - spread).floor(),
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
pub(crate) fn glyph_field_in(commands: &[Command], area: FieldBox) -> Option<FieldImage> {
    let segments = flatten(commands);
    if segments.is_empty() {
        return None;
    }

    let spread = FIELD_SPREAD_PX as f32;
    let left = area.left;
    let top = area.top;
    let width = (area.right - area.left).max(1.0) as u32;
    let height = (area.top - area.bottom).max(1.0) as u32;

    let mut data = Vec::with_capacity((width * height) as usize);
    for row in 0..height {
        // Sampled at the centre of the texel, and downwards: the field is read
        // as an image, where the first row is the top, while the outline it is
        // measured from counts y upwards from the baseline.
        let y = top - row as f32 - 0.5;
        for column in 0..width {
            let x = left + column as f32 + 0.5;
            let mut nearest = f32::MAX;
            let mut winding = 0;
            for segment in &segments {
                nearest = nearest.min(segment.distance_squared(x, y));
                winding += segment.winding(x, y);
            }
            let distance = nearest.sqrt();
            // Negative inside, positive outside, then folded into the byte the
            // shader thresholds: zero is deep inside a stroke and one is
            // further out than the spread reaches.
            let signed = if winding != 0 { -distance } else { distance };
            let unit = (signed + spread) / (spread * 2.0);
            data.push((unit.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }

    Some(FieldImage {
        left: left as i32,
        top: top as i32,
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
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) data: Rc<Vec<u8>>,
}

/// How much of the two shapes fails to overlap, from nothing to all of it.
///
/// The area covered by one and not the other, over the area covered by either.
/// Zero for a letter measured against itself, small for two that differ in one
/// stroke, large for two with nothing in common — which is exactly the order in
/// which linear interpolation between them starts to come apart, so it is what
/// decides how hard the renderer has to work to hold the shape together.
pub(crate) fn disagreement(first: &FieldImage, second: &FieldImage) -> f32 {
    // Below the halfway byte is inside the letter, which is the direction the
    // shader thresholds in.
    const INK: u8 = 128;
    let mut apart = 0_u32;
    let mut either = 0_u32;
    for (left, right) in first.data.iter().zip(second.data.iter()) {
        let (inside, other) = (*left < INK, *right < INK);
        if inside || other {
            either += 1;
            if inside != other {
                apart += 1;
            }
        }
    }
    if either == 0 {
        return 0.0;
    }
    apart as f32 / either as f32
}
