// Turning one letter into another.
//
// Averaging two distance fields is not a morph. It has no idea which stroke of
// one corresponds to which stroke of the other, so it moves every contour
// towards whatever happens to be nearest; where the two letters agree that is
// exact, and where they do not the result swells and tears. That is a known
// property of interpolating distance fields, not something to tune away.
//
// Correspondence belongs in the outline, where the strokes actually are. Both
// letters are broken into closed contours, each contour is resampled to the
// same number of points spaced evenly along its length, the contours of one
// letter are matched to the contours of the other, and each pair is rotated to
// the offset that lines their points up best. After that the morph is the
// obvious thing: every point walks to its opposite number, and the shape in
// between is a real outline that can be measured like any other.
//
// The result is one technique for every pair. There is no case where two
// letters are too unalike to morph, because nothing here depends on them
// resembling each other.

use cosmic_text::Command;

use crate::glyph_fields::{Segment, flatten};

/// How many points each contour is resampled to.
///
/// Enough that a letter's corners survive the resampling, few enough that
/// matching two contours is a handful of arithmetic per pair. Points are spaced
/// by arc length rather than by parameter, so a long straight stem gets as many
/// as a tight curve of the same length and neither is favoured when they meet.
const CONTOUR_POINTS: usize = 96;

/// One closed loop of an outline, resampled and measured.
pub(crate) struct Contour {
    points: Vec<(f32, f32)>,
    /// Signed, so it also says which way the loop is wound: a letter's counter
    /// is wound against its body, and the two must not be matched to each other.
    area: f32,
    centre: (f32, f32),
}

/// Breaks an outline into closed loops of evenly spaced points.
pub(crate) fn contours(commands: &[Command]) -> Vec<Contour> {
    let mut loops: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let mut last: Option<(f32, f32)> = None;

    // `flatten` closes every subpath, so a loop ends wherever the next piece
    // does not continue from the last one.
    for segment in flatten(commands) {
        let start = (segment.x0, segment.y0);
        if last.is_some_and(|point| distance(point, start) > 0.01) {
            if current.len() > 2 {
                loops.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
        current.push(start);
        last = Some((segment.x1, segment.y1));
    }
    if current.len() > 2 {
        loops.push(current);
    }

    loops
        .into_iter()
        .filter_map(|points| {
            let points = resample(&points, CONTOUR_POINTS)?;
            let area = signed_area(&points);
            Some(Contour {
                centre: centre_of(&points),
                area,
                points,
            })
        })
        .collect()
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
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

fn centre_of(points: &[(f32, f32)]) -> (f32, f32) {
    let mut sum = (0.0, 0.0);
    for point in points {
        sum.0 += point.0;
        sum.1 += point.1;
    }
    let count = points.len().max(1) as f32;
    (sum.0 / count, sum.1 / count)
}

/// Walks a closed loop and drops `count` points on it at equal distances.
fn resample(points: &[(f32, f32)], count: usize) -> Option<Vec<(f32, f32)>> {
    if points.len() < 3 {
        return None;
    }
    let mut lengths = Vec::with_capacity(points.len());
    let mut total = 0.0;
    for index in 0..points.len() {
        let next = points[(index + 1) % points.len()];
        total += distance(points[index], next);
        lengths.push(total);
    }
    if total <= f32::EPSILON {
        return None;
    }

    let mut walked = Vec::with_capacity(count);
    let mut edge = 0usize;
    for step in 0..count {
        let target = total * step as f32 / count as f32;
        while edge + 1 < lengths.len() && lengths[edge] < target {
            edge += 1;
        }
        let before = if edge == 0 { 0.0 } else { lengths[edge - 1] };
        let span = (lengths[edge] - before).max(f32::EPSILON);
        let along = ((target - before) / span).clamp(0.0, 1.0);
        let from = points[edge];
        let to = points[(edge + 1) % points.len()];
        walked.push((
            from.0 + (to.0 - from.0) * along,
            from.1 + (to.1 - from.1) * along,
        ));
    }
    Some(walked)
}

/// Two contours matched up, ready to be walked between.
pub(crate) struct Paired {
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
pub(crate) fn pair_up(from: Vec<Contour>, to: Vec<Contour>) -> Vec<Paired> {
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
pub(crate) fn between(paired: &[Paired], travel: f32) -> Vec<Segment> {
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
