//! Making `evenodd` mean what `nonzero` means.
//!
//! A field decides inside from outside by counting how many times the boundary
//! winds around the point. That is the `nonzero` rule exactly. Under `evenodd`
//! the same drawing means something else: a point is inside when it is enclosed
//! an odd number of times, whichever way round each loop happens to be wound.
//!
//! The two agree on any drawing where the loops already alternate direction by
//! depth, which is most of them — and where they do not, turning the offending
//! loops around makes them agree, without changing what the drawing means. So
//! that is what happens here: every loop is asked how deeply it is nested, and
//! is wound to match.

use morf_outline::{Step, contours};

/// Re-winds the loops of one path so a winding count reads it as even-odd did.
///
/// Rebuilt rather than turned around in place: a loop traversed the other way
/// needs one step more than it arrived with — the move it starts from is no
/// longer the end of anything, so the edge back to it has to be written out.
pub(crate) fn by_nesting(steps: &[Step]) -> Vec<Step> {
    let spans = subpaths(steps);
    if spans.len() < 2 {
        return steps.to_vec();
    }
    let outlines: Vec<Vec<(f32, f32)>> = spans
        .iter()
        .map(|span| flatten_span(&steps[span.clone()]))
        .collect();
    let mut turn = Vec::with_capacity(spans.len());
    for (index, points) in outlines.iter().enumerate() {
        let Some(inside) = points.first().copied() else {
            turn.push(false);
            continue;
        };
        let depth = outlines
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .filter(|(_, around)| encloses(around, inside))
            .count();
        // Odd depth is a hole, which must be wound against its container.
        let wants_clockwise = depth % 2 == 1;
        turn.push(wants_clockwise != (signed_area(points) < 0.0));
    }
    let mut rewound = Vec::with_capacity(steps.len() + spans.len() * 2);
    for (span, turn) in spans.into_iter().zip(turn) {
        if turn {
            rewound.extend(reversed(&steps[span]));
        } else {
            rewound.extend_from_slice(&steps[span]);
        }
    }
    rewound
}

/// Where each closed loop starts and ends in the step list.
fn subpaths(steps: &[Step]) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, step) in steps.iter().enumerate() {
        if matches!(step, Step::Move(_, _)) && index > start {
            spans.push(start..index);
            start = index;
        }
    }
    if start < steps.len() {
        spans.push(start..steps.len());
    }
    spans
}

/// The loop as a polygon, only accurate enough to answer "is this inside".
fn flatten_span(steps: &[Step]) -> Vec<(f32, f32)> {
    contours(steps)
        .first()
        .map(|contour| morf_outline::contour_of(contour).to_vec())
        .unwrap_or_default()
}

fn signed_area(points: &[(f32, f32)]) -> f32 {
    let mut total = 0.0;
    for index in 0..points.len() {
        let (x0, y0) = points[index];
        let (x1, y1) = points[(index + 1) % points.len()];
        total += x0 * y1 - x1 * y0;
    }
    total * 0.5
}

fn encloses(points: &[(f32, f32)], at: (f32, f32)) -> bool {
    let mut inside = false;
    for index in 0..points.len() {
        let (x0, y0) = points[index];
        let (x1, y1) = points[(index + 1) % points.len()];
        if (y0 <= at.1) != (y1 <= at.1) {
            let t = (at.1 - y0) / (y1 - y0);
            if x0 + t * (x1 - x0) > at.0 {
                inside = !inside;
            }
        }
    }
    inside
}

/// One loop with the same shape and the opposite winding.
///
/// A curve reversed keeps its control points; they are simply passed in the
/// other order, because the curve is being travelled the other way.
fn reversed(steps: &[Step]) -> Vec<Step> {
    let mut points = Vec::with_capacity(steps.len());
    let mut at = match steps.first() {
        Some(Step::Move(x, y)) => (*x, *y),
        _ => return steps.to_vec(),
    };
    let first = at;
    for step in steps.iter().skip(1) {
        match *step {
            Step::Line(x, y) => {
                points.push((Turn::Line, at, (x, y)));
                at = (x, y);
            }
            Step::Quad(cx, cy, x, y) => {
                points.push((Turn::Quad(cx, cy), at, (x, y)));
                at = (x, y);
            }
            Step::Cubic(ax, ay, bx, by, x, y) => {
                points.push((Turn::Cubic(ax, ay, bx, by), at, (x, y)));
                at = (x, y);
            }
            Step::Move(..) | Step::Close => {}
        }
    }
    if at != first {
        points.push((Turn::Line, at, first));
    }

    let mut rebuilt = Vec::with_capacity(steps.len());
    rebuilt.push(Step::Move(first.0, first.1));
    for (kind, from, _) in points.iter().rev() {
        rebuilt.push(match *kind {
            Turn::Line => Step::Line(from.0, from.1),
            // The controls are walked in the other order too.
            Turn::Quad(cx, cy) => Step::Quad(cx, cy, from.0, from.1),
            Turn::Cubic(ax, ay, bx, by) => Step::Cubic(bx, by, ax, ay, from.0, from.1),
        });
    }
    rebuilt.push(Step::Close);
    rebuilt
}

enum Turn {
    Line,
    Quad(f32, f32),
    Cubic(f32, f32, f32, f32),
}
