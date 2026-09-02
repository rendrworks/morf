//! Breaking an outline into closed loops of evenly spaced points, which is what
//! makes one shape able to turn into another.
//!
//! Averaging two distance fields is not a morph. It has no idea which stroke of
//! one corresponds to which stroke of the other, so it moves every contour
//! towards whatever happens to be nearest; where the two shapes agree that is
//! exact, and where they do not the result swells and tears. That is a known
//! property of interpolating distance fields, not something to tune away.
//!
//! Correspondence belongs in the outline, where the strokes actually are. Both
//! shapes are broken into closed contours, each contour is resampled to the
//! same number of points spaced evenly along its length, the contours of one
//! are matched to the contours of the other, and each pair is rotated to
//! the offset that lines their points up best. After that the morph is the
//! obvious thing: every point walks to its opposite number, and the shape in
//! between is a real outline that can be measured like any other.
//!
//! The result is one technique for every pair. There is no case where two
//! outlines are too unalike to morph, because nothing here depends on them
//! resembling each other — which is also why a letter and a drawing morph on
//! exactly the same terms.

use crate::corners::corner_points;
use crate::flatten::flatten;
use crate::step::Step;

/// How many points each contour is resampled to.
///
/// Enough that a letter's corners survive the resampling, few enough that
/// matching two contours is a handful of arithmetic per pair. Points are spaced
/// by arc length rather than by parameter, so a long straight stem gets as many
/// as a tight curve of the same length and neither is favoured when they meet.
pub const CONTOUR_POINTS: usize = 96;

/// One closed loop of an outline, resampled and measured.
#[derive(Clone, Debug)]
pub struct Contour {
    pub(crate) points: Vec<(f32, f32)>,
    /// Signed, so it also says which way the loop is wound: a letter's counter
    /// is wound against its body, and the two must not be matched to each other.
    pub(crate) area: f32,
    pub(crate) centre: (f32, f32),
}

/// Breaks an outline into closed loops of evenly spaced points, with a point
/// kept on every corner.
pub fn contours(steps: &[Step]) -> Vec<Contour> {
    // Where the letter actually turns, taken from the curves before they were
    // broken into pieces. Spacing points evenly along a loop puts none of them
    // on a corner except by luck, so `W` came back with its apex chamfered by
    // nine pixels at a headline size while `o`, which has no corner to lose,
    // came back exact.
    let sharp = corner_points(steps);
    let mut loops: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let mut last: Option<(f32, f32)> = None;

    // `flatten` closes every subpath, so a loop ends wherever the next piece
    // does not continue from the last one.
    for segment in flatten(steps) {
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
        .enumerate()
        .filter_map(|(index, points)| {
            // A corner belongs to the loop whose flattened vertices it sits on,
            // which is exact: a corner *is* one of those vertices, being where
            // two curves meet. Matching that way rather than by position in the
            // list survives a subpath being dropped for having no area.
            let corners: Vec<_> = sharp
                .get(index)
                .into_iter()
                .flatten()
                .copied()
                .filter(|corner| points.iter().any(|point| distance(*point, *corner) < 0.01))
                .collect();
            let points = resample(&points, CONTOUR_POINTS, &corners)?;
            let area = signed_area(&points);
            Some(Contour {
                centre: centre_of(&points),
                area,
                points,
            })
        })
        .collect()
}

/// The points of one contour, for the diagnostics — which live in another
/// crate, so this cannot be test-only.
pub fn contour_of(contour: &Contour) -> &[(f32, f32)] {
    &contour.points
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
pub fn resample(
    points: &[(f32, f32)],
    count: usize,
    corners: &[(f32, f32)],
) -> Option<Vec<(f32, f32)>> {
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

    // Pull the nearest walked point onto each corner, rather than inserting one.
    //
    // Every contour has to come back the same length: the correspondence
    // between two letters is positional, and `pair_up`, `best_offset` and
    // `walk` all read the two lists in step. Snapping keeps the count and gives
    // up a little of the even spacing locally, which nothing depends on — and
    // it improves the correspondence rather than costing it, because corner
    // *i* of one letter now lands on corner *i* of the other, so what lies
    // between them keeps a corner too.
    for corner in corners {
        let mut nearest = 0;
        let mut best = f32::MAX;
        for (index, point) in walked.iter().enumerate() {
            let apart = distance(*point, *corner);
            if apart < best {
                best = apart;
                nearest = index;
            }
        }
        walked[nearest] = *corner;
    }
    Some(walked)
}
