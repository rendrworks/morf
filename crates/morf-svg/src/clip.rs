//! `clip-path`, which is an intersection.
//!
//! A clipped group is the group's shape *and* the clip's shape — everywhere the
//! two overlap, and nowhere else. A field does intersections natively, so the
//! honest thing would be to hand both regions over and let it do that; but a
//! polygon layer holds one outline, and one outline cannot say "and also". So
//! the intersection is taken here, on the points, before the outline leaves.
//!
//! Two cases are exact and are the ones that occur:
//!
//! A clip that already contains everything it is applied to changes nothing, so
//! it is dropped. This is not a shortcut — it is the answer — and it covers the
//! full-viewBox rectangle that Material, Figma and every other exporter wraps
//! its icons in.
//!
//! A convex clip — a rectangle, a rounded rectangle, a circle, a hexagon — is
//! clipped against exactly, by Sutherland and Hodgman's algorithm, which is
//! exact for a convex window and is why the window has to be convex.
//!
//! Anything else is refused by name. A concave or disjoint clip needs general
//! polygon intersection, and half of one silently applied is worse than a file
//! that says it was not understood: the drawing would come out whole, which is
//! the one answer that is certainly wrong.

use morf_outline::{Step, contour_of, contours};
use resvg::usvg;

use crate::{steps_of, walk};

/// A clip's own shape, as closed polygons in the document's coordinates.
pub(crate) type Region = Vec<Vec<(f32, f32)>>;

/// Why a clip could not be applied, for an error that names the file.
pub(crate) struct Unsupported;

/// The region a clip permits, with any clip of its own already taken into it.
pub(crate) fn region(clip: &usvg::ClipPath) -> Result<Region, Unsupported> {
    let mut steps = Vec::new();
    walk::group_into(clip.root(), &mut steps);
    let mut region: Region = contours(&steps)
        .iter()
        .map(|contour| contour_of(contour).to_vec())
        .collect();
    if region.is_empty() {
        return Err(Unsupported);
    }
    // A clip may itself be clipped, and the two intersect. Taking the nested
    // one only when it changes nothing keeps this exact; anything else is
    // general polygon intersection again, and is refused rather than guessed.
    if let Some(nested) = clip.clip_path() {
        let inner = self::region(nested)?;
        if !encloses(&inner, &region) {
            let Some(window) = convex_window(&inner) else {
                return Err(Unsupported);
            };
            region = region
                .into_iter()
                .filter_map(|contour| clip_to(&contour, &window))
                .collect();
            if region.is_empty() {
                return Err(Unsupported);
            }
        }
    }
    Ok(region)
}

/// The outline of a group, kept only where a clip allows it.
pub(crate) fn apply(content: &[Step], region: &Region) -> Result<Vec<Step>, Unsupported> {
    let loops: Vec<Vec<(f32, f32)>> = contours(content)
        .iter()
        .map(|contour| contour_of(contour).to_vec())
        .collect();
    if loops.is_empty() {
        return Ok(Vec::new());
    }
    // The clip already contains everything, so it says nothing. The curves are
    // handed back untouched — the flattening above was only to ask the question.
    if encloses(region, &loops) {
        return Ok(content.to_vec());
    }
    let Some(window) = convex_window(region) else {
        return Err(Unsupported);
    };
    let mut clipped = Vec::new();
    for contour in &loops {
        let Some(kept) = clip_to(contour, &window) else {
            continue;
        };
        polygon_steps(&kept, &mut clipped);
    }
    Ok(clipped)
}

/// Whether a region contains every point of some loops.
///
/// By winding, not by "inside any one of them", so a clip that is a ring is
/// understood to have a hole in it rather than being taken as solid.
fn encloses(region: &Region, loops: &[Vec<(f32, f32)>]) -> bool {
    loops
        .iter()
        .flatten()
        .all(|point| winding(region, *point) != 0)
}

fn winding(region: &Region, at: (f32, f32)) -> i32 {
    let mut total = 0;
    for contour in region {
        for index in 0..contour.len() {
            let (x0, y0) = contour[index];
            let (x1, y1) = contour[(index + 1) % contour.len()];
            if (y0 <= at.1) != (y1 <= at.1) {
                let t = (at.1 - y0) / (y1 - y0);
                if x0 + t * (x1 - x0) > at.0 {
                    total += if y1 > y0 { 1 } else { -1 };
                }
            }
        }
    }
    total
}

/// A region as one convex polygon, wound anticlockwise, or nothing.
fn convex_window(region: &Region) -> Option<Vec<(f32, f32)>> {
    let [contour] = region.as_slice() else {
        return None;
    };
    if contour.len() < 3 {
        return None;
    }
    let mut seen = 0i32;
    for index in 0..contour.len() {
        let a = contour[index];
        let b = contour[(index + 1) % contour.len()];
        let c = contour[(index + 2) % contour.len()];
        let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        // A resampled loop has near-collinear neighbours all round it, and a
        // sign taken from rounding noise would call every circle concave.
        if cross.abs() < 1e-6 {
            continue;
        }
        let sign = if cross > 0.0 { 1 } else { -1 };
        if seen == 0 {
            seen = sign;
        } else if seen != sign {
            return None;
        }
    }
    let mut window = contour.clone();
    if seen < 0 {
        window.reverse();
    }
    Some(window)
}

/// Sutherland and Hodgman: the subject cut against each edge of the window in
/// turn. Exact for a convex window, which is the whole of why one is required —
/// against a concave window this leaves degenerate seams along the cuts.
fn clip_to(subject: &[(f32, f32)], window: &[(f32, f32)]) -> Option<Vec<(f32, f32)>> {
    let mut kept = subject.to_vec();
    for index in 0..window.len() {
        if kept.is_empty() {
            return None;
        }
        let a = window[index];
        let b = window[(index + 1) % window.len()];
        let inside = |point: (f32, f32)| {
            (b.0 - a.0) * (point.1 - a.1) - (b.1 - a.1) * (point.0 - a.0) >= 0.0
        };
        let mut next = Vec::with_capacity(kept.len() + 4);
        for step in 0..kept.len() {
            let from = kept[step];
            let to = kept[(step + 1) % kept.len()];
            let (from_in, to_in) = (inside(from), inside(to));
            if from_in {
                next.push(from);
            }
            if from_in != to_in {
                next.push(crossing(from, to, a, b));
            }
        }
        kept = next;
    }
    (kept.len() >= 3).then_some(kept)
}

fn crossing(from: (f32, f32), to: (f32, f32), a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let (ex, ey) = (b.0 - a.0, b.1 - a.1);
    let denominator = ex * dy - ey * dx;
    if denominator.abs() < 1e-12 {
        return from;
    }
    let t = (ex * (a.1 - from.1) - ey * (a.0 - from.0)) / denominator;
    (from.0 + dx * t, from.1 + dy * t)
}

fn polygon_steps(points: &[(f32, f32)], into: &mut Vec<Step>) {
    let Some(first) = points.first() else {
        return;
    };
    into.push(Step::Move(first.0, first.1));
    for point in &points[1..] {
        into.push(Step::Line(point.0, point.1));
    }
    into.push(Step::Close);
}

/// A clip's shapes, placed, for the region walk.
pub(crate) fn shape_into(path: &usvg::Path, into: &mut Vec<Step>) {
    if let Some(placed) = path.data().clone().transform(path.abs_transform()) {
        steps_of(&placed, into);
    }
}
