//! Outlines broken into straight pieces.
//!
//! Everything a field measures is a distance to a straight edge, so the curves
//! come apart first — finely enough that the chord is nearer the curve than the
//! stored byte can tell.

use crate::step::Step;

/// In reference pixels of allowed deviation. The distance to a chord this close
/// to the curve is wrong by less than the eighth of a pixel the stored byte can
/// express, so nothing is gained by going finer.
const FLATTEN_TOLERANCE: f32 = 0.05;

/// One straight piece of an outline.
pub struct Segment {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Segment {
    /// Distance from a point to this piece, squared.
    pub fn distance_squared(&self, x: f32, y: f32) -> f32 {
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
    /// Kept for the brute-force reference a fast generator is checked against.
    /// A generator that sweeps a whole row at a time never asks this.
    pub fn winding(&self, x: f32, y: f32) -> i32 {
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
/// leave a gap the winding count could escape through and the shape would come
/// back inside-out. Fonts close their contours and well-formed SVG paths do
/// too, but a move without a close before it is legal in both and has to be
/// treated as one.
pub fn flatten(steps: &[Step]) -> Vec<Segment> {
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

    for step in steps {
        match *step {
            Step::Move(x, y) => {
                line(at, start, &mut segments);
                start = (x, y);
                at = start;
            }
            Step::Line(x, y) => {
                let to = (x, y);
                line(at, to, &mut segments);
                at = to;
            }
            Step::Quad(cx, cy, x, y) => {
                let to = (x, y);
                for (from, next) in quadratic_steps(at, (cx, cy), to) {
                    line(from, next, &mut segments);
                }
                at = to;
            }
            Step::Cubic(ax, ay, bx, by, x, y) => {
                let to = (x, y);
                for (from, next) in cubic_steps(at, (ax, ay), (bx, by), to) {
                    line(from, next, &mut segments);
                }
                at = to;
            }
            Step::Close => {
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
