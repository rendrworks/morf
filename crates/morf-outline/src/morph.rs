//! Walking one outline onto another.
//!
//! Two outlines correspond when their loops are paired by where they sit, each
//! resampled to the same count, and each rotated until its points line up with
//! its opposite number. Nothing in here knows whether a loop came out of a font
//! or a drawing, which is why a letter can walk onto an icon.

use crate::contours::{CONTOUR_POINTS, Contour};
use crate::flatten::Segment;

pub struct Paired {
    from: Vec<(f32, f32)>,
    to: Vec<(f32, f32)>,
}

/// Matches the loops of one letter to the loops of another.
///
/// Largest first, and bodies only ever to bodies: a counter is wound against
/// the shape that contains it, so pairing one with a body would turn the letter
/// inside out on the way across. A loop with nothing to match — the hole in `o`
/// when it becomes `l` — is paired with a copy of itself collapsed to its own
/// centre, so it closes up smoothly instead of vanishing between two frames.
pub fn pair_up(from: Vec<Contour>, to: Vec<Contour>) -> Vec<Paired> {
    let (from_bodies, from_holes) = split_wound(from);
    let (to_bodies, to_holes) = split_wound(to);
    let mut paired = Vec::new();
    match_group(&from_bodies, &to_bodies, &mut paired);
    match_group(&from_holes, &to_holes, &mut paired);
    paired
}

fn split_wound(contours: Vec<Contour>) -> (Vec<Contour>, Vec<Contour>) {
    let (mut bodies, mut holes): (Vec<_>, Vec<_>) = contours
        .into_iter()
        .partition(|contour| contour.area >= 0.0);
    bodies.sort_by(|a, b| b.area.abs().total_cmp(&a.area.abs()));
    holes.sort_by(|a, b| b.area.abs().total_cmp(&a.area.abs()));
    (bodies, holes)
}

fn match_group(from: &[Contour], to: &[Contour], into: &mut Vec<Paired>) {
    let count = from.len().max(to.len());
    for index in 0..count {
        match (from.get(index), to.get(index)) {
            (Some(a), Some(b)) => {
                let shift = best_offset(&a.points, &b.points);
                into.push(Paired {
                    from: a.points.clone(),
                    to: rotated(&b.points, shift),
                });
            }
            // Nothing opposite it: it closes to a point where it stands, and
            // the letter loses that stroke smoothly rather than by having it
            // switched off.
            (Some(a), None) => into.push(Paired {
                from: a.points.clone(),
                to: vec![a.centre; a.points.len()],
            }),
            (None, Some(b)) => into.push(Paired {
                from: vec![b.centre; b.points.len()],
                to: b.points.clone(),
            }),
            (None, None) => {}
        }
    }
}

fn rotated(points: &[(f32, f32)], shift: usize) -> Vec<(f32, f32)> {
    (0..points.len())
        .map(|index| points[(index + shift) % points.len()])
        .collect()
}

/// The rotation of one loop against the other that lines their points up best.
///
/// Both are closed, so where the walk began is arbitrary and any offset is as
/// valid as any other — but only one of them puts the top of a letter over the
/// top of the other. Without it every morph starts by unwinding a whole turn,
/// which is the difference between a stroke moving and a shape spinning apart.
fn best_offset(from: &[(f32, f32)], to: &[(f32, f32)]) -> usize {
    let count = from.len().min(to.len());
    if count == 0 {
        return 0;
    }
    // Every eighth offset, then refined around the winner: the cost varies
    // smoothly with the offset, so a coarse pass finds the right basin for an
    // eighth of the work.
    let mut best = (f32::MAX, 0usize);
    let score = |shift: usize| -> f32 {
        let mut total = 0.0;
        let mut index = 0;
        while index < count {
            let a = from[index];
            let b = to[(index + shift) % count];
            total += (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);
            index += 4;
        }
        total
    };
    let mut shift = 0;
    while shift < count {
        let cost = score(shift);
        if cost < best.0 {
            best = (cost, shift);
        }
        shift += 8;
    }
    let around = best.1;
    for offset in 0..16 {
        let shift = (around + count + offset - 8) % count;
        let cost = score(shift);
        if cost < best.0 {
            best = (cost, shift);
        }
    }
    best.1
}

/// The outline part way between the two letters.
pub fn between(paired: &[Paired], travel: f32) -> Vec<Segment> {
    let mut segments = Vec::new();
    for pair in paired {
        let count = pair.from.len().min(pair.to.len());
        if count < 3 {
            continue;
        }
        let point = |index: usize| {
            let a = pair.from[index % count];
            let b = pair.to[index % count];
            (a.0 + (b.0 - a.0) * travel, a.1 + (b.1 - a.1) * travel)
        };
        for index in 0..count {
            let (x0, y0) = point(index);
            let (x1, y1) = point(index + 1);
            if (x0 - x1).abs() > f32::EPSILON || (y0 - y1).abs() > f32::EPSILON {
                segments.push(Segment { x0, y0, x1, y1 });
            }
        }
    }
    segments
}

/// The points of every contour, end to end.
///
/// Each run is exactly `CONTOUR_POINTS` long, because that is what resampling
/// guarantees — which is what lets one layer hold a letter with a counter in
/// it: the shader closes each run on itself and sums the windings, so `8` comes
/// out with two holes rather than as one loop threaded through itself.
pub fn contour_points(contours: &[Contour]) -> Vec<(f32, f32)> {
    let mut points = Vec::with_capacity(contours.len() * CONTOUR_POINTS);
    for contour in contours {
        points.extend_from_slice(&contour.points);
    }
    points
}

/// The same, part way towards another letter.
pub fn walk(paired: &[Paired], travel: f32) -> Vec<(f32, f32)> {
    let mut points = Vec::with_capacity(paired.len() * CONTOUR_POINTS);
    for pair in paired {
        let count = pair.from.len().min(pair.to.len());
        for index in 0..count {
            let a = pair.from[index];
            let b = pair.to[index];
            points.push((a.0 + (b.0 - a.0) * travel, a.1 + (b.1 - a.1) * travel));
        }
    }
    points
}
