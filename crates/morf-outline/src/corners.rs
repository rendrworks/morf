//! Where an outline turns sharply, taken from the curves rather than from the
//! pieces they were broken into.
//!
//! A corner has to be found before flattening, not after. Once an outline is
//! chopped into straight pieces at a twentieth of a pixel, every join along a
//! tight curve turns by a few degrees, and an angle test applied to those pieces
//! calls all of them corners — which is how an `o` ends up with ninety-six of
//! them. The curves know better: a Bézier's direction at each end comes from its
//! control points, and two curves meeting either continue each other or do not.

use crate::step::Step;

/// How far two curves must turn away from each other to be a corner.
///
/// The cosine of about eight degrees. Well above what a curve's own control
/// points produce where it joins its neighbour smoothly, well below the
/// shallowest corner a letterform or a drawn shape has.
const CORNER_COSINE: f32 = 0.99;

/// A corner, and the two half-planes that meet there.
///
/// Kept for the diagnostic that rejected it. Reconstructing the crease from
/// these was measured against six differently-built faces and made the field
/// *worse* on every one — see `probe_corner_cell_error`. It stays so that the
/// measurement can be re-run rather than re-argued.
///
/// A half-plane's distance is affine, and bilinear interpolation reproduces an
/// affine function exactly — which is the whole reason this is worth carrying.
/// What bilinear cannot reproduce is the crease where two of them meet, and
/// that is precisely what these two let the shader rebuild.
///
/// Public rather than test-only because the measurement lives in another crate:
/// `#[cfg(test)]` does not reach across one.
#[derive(Clone, Copy, Debug)]
pub struct Corner {
    /// Where the two curves meet.
    pub at: (f32, f32),
    /// Outward normals of the arriving and leaving edges.
    pub normals: [(f32, f32); 2],
    /// Whether the shape is locally the *intersection* of the two half-planes
    /// rather than their union — `max` of the two distances rather than `min`.
    /// Getting this backwards turns a corner into a notch.
    pub convex: bool,
}

impl Corner {
    /// The distance to the wedge these two half-planes cut out.
    ///
    /// Beyond the tip of a convex corner — outside both half-planes at once —
    /// the nearest point of the shape is the corner itself, so the distance is
    /// to the point and not to either line. Taking the larger of the two there
    /// understates it by up to the difference between a side and a diagonal,
    /// which is a corner rounded off by another name. A reflex corner is the
    /// same statement inside out.
    pub fn distance(&self, x: f32, y: f32) -> f32 {
        let (dx, dy) = (x - self.at.0, y - self.at.1);
        let first = dx * self.normals[0].0 + dy * self.normals[0].1;
        let second = dx * self.normals[1].0 + dy * self.normals[1].1;
        let to_tip = (dx * dx + dy * dy).sqrt();
        if self.convex {
            if first > 0.0 && second > 0.0 {
                to_tip
            } else {
                first.max(second)
            }
        } else if first < 0.0 && second < 0.0 {
            -to_tip
        } else {
            first.min(second)
        }
    }
}

/// The corners of each closed subpath, with the half-planes meeting at them.
///
/// Diagnostic only; see `Corner`.
pub fn corners(steps: &[Step]) -> Vec<Corner> {
    let mut found = Vec::new();
    for edges in split_subpaths(steps) {
        if edges.len() < 2 {
            continue;
        }
        // Which way the loop is wound decides which side of an edge is outside,
        // and a letter's counters are wound against its body.
        let mut area = 0.0;
        for index in 0..edges.len() {
            let a = edges[index].start;
            let b = edges[(index + 1) % edges.len()].start;
            area += a.0 * b.1 - b.0 * a.1;
        }
        let anticlockwise = area > 0.0;
        let outward = |direction: (f32, f32)| {
            if anticlockwise {
                (direction.1, -direction.0)
            } else {
                (-direction.1, direction.0)
            }
        };
        for index in 0..edges.len() {
            let previous = &edges[(index + edges.len() - 1) % edges.len()];
            let here = &edges[index];
            if !turns(previous.leaving, here.entering) {
                continue;
            }
            let cross = previous.leaving.0 * here.entering.1 - previous.leaving.1 * here.entering.0;
            found.push(Corner {
                at: here.start,
                normals: [outward(previous.leaving), outward(here.entering)],
                convex: (cross > 0.0) == anticlockwise,
            });
        }
    }
    found
}

/// The corner vertices of one closed subpath, in the order they are walked.
pub fn corner_points(steps: &[Step]) -> Vec<Vec<(f32, f32)>> {
    let mut subpaths = Vec::new();
    for edges in split_subpaths(steps) {
        if edges.len() < 2 {
            // One edge closing on itself has a join, but nothing to compare it
            // against that is not itself.
            subpaths.push(Vec::new());
            continue;
        }
        let mut corners = Vec::new();
        for index in 0..edges.len() {
            let previous = &edges[(index + edges.len() - 1) % edges.len()];
            let here = &edges[index];
            if turns(previous.leaving, here.entering) {
                corners.push(here.start);
            }
        }
        subpaths.push(corners);
    }
    subpaths
}

/// Whether two directions part company sharply enough to be a corner.
fn turns(leaving: (f32, f32), entering: (f32, f32)) -> bool {
    let dot = leaving.0 * entering.0 + leaving.1 * entering.1;
    let cross = leaving.0 * entering.1 - leaving.1 * entering.0;
    dot <= 0.0 || cross.abs() > (1.0 - CORNER_COSINE * CORNER_COSINE).sqrt()
}

/// One curve of an outline: where it starts, and which way it points at each end.
struct Edge {
    start: (f32, f32),
    entering: (f32, f32),
    leaving: (f32, f32),
}

fn normalise(x: f32, y: f32) -> (f32, f32) {
    let length = (x * x + y * y).sqrt();
    if length <= f32::EPSILON {
        (0.0, 0.0)
    } else {
        (x / length, y / length)
    }
}

/// A curve's direction at each end, from its control points.
///
/// A control point that sits on the endpoint it belongs to says nothing about
/// the direction there, so the next one along answers instead — which is what
/// the degenerate cases below are for.
fn quadratic_ends(
    from: (f32, f32),
    control: (f32, f32),
    to: (f32, f32),
) -> ((f32, f32), (f32, f32)) {
    let entering = normalise(control.0 - from.0, control.1 - from.1);
    let leaving = normalise(to.0 - control.0, to.1 - control.1);
    let chord = normalise(to.0 - from.0, to.1 - from.1);
    (
        if entering == (0.0, 0.0) {
            chord
        } else {
            entering
        },
        if leaving == (0.0, 0.0) {
            chord
        } else {
            leaving
        },
    )
}

fn cubic_ends(
    from: (f32, f32),
    first: (f32, f32),
    second: (f32, f32),
    to: (f32, f32),
) -> ((f32, f32), (f32, f32)) {
    let chord = normalise(to.0 - from.0, to.1 - from.1);
    let mut entering = normalise(first.0 - from.0, first.1 - from.1);
    if entering == (0.0, 0.0) {
        entering = normalise(second.0 - from.0, second.1 - from.1);
    }
    let mut leaving = normalise(to.0 - second.0, to.1 - second.1);
    if leaving == (0.0, 0.0) {
        leaving = normalise(to.0 - first.0, to.1 - first.1);
    }
    (
        if entering == (0.0, 0.0) {
            chord
        } else {
            entering
        },
        if leaving == (0.0, 0.0) {
            chord
        } else {
            leaving
        },
    )
}

/// Breaks the command stream into closed subpaths of curves.
///
/// Every subpath is closed, because a distance is measured to a boundary and a
/// boundary with a gap in it is not one. A move before an explicit close
/// closes the previous subpath itself.
fn split_subpaths(steps: &[Step]) -> Vec<Vec<Edge>> {
    let mut subpaths = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut start = (0.0_f32, 0.0_f32);
    let mut at = start;

    let close = |edges: &mut Vec<Edge>, at: (f32, f32), start: (f32, f32)| {
        if at != start {
            let direction = normalise(start.0 - at.0, start.1 - at.1);
            edges.push(Edge {
                start: at,
                entering: direction,
                leaving: direction,
            });
        }
    };

    for step in steps {
        match *step {
            Step::Move(x, y) => {
                close(&mut edges, at, start);
                if !edges.is_empty() {
                    subpaths.push(std::mem::take(&mut edges));
                }
                start = (x, y);
                at = start;
            }
            Step::Line(x, y) => {
                let to = (x, y);
                let direction = normalise(to.0 - at.0, to.1 - at.1);
                edges.push(Edge {
                    start: at,
                    entering: direction,
                    leaving: direction,
                });
                at = to;
            }
            Step::Quad(cx, cy, x, y) => {
                let to = (x, y);
                let (entering, leaving) = quadratic_ends(at, (cx, cy), to);
                edges.push(Edge {
                    start: at,
                    entering,
                    leaving,
                });
                at = to;
            }
            Step::Cubic(ax, ay, bx, by, x, y) => {
                let to = (x, y);
                let (entering, leaving) = cubic_ends(at, (ax, ay), (bx, by), to);
                edges.push(Edge {
                    start: at,
                    entering,
                    leaving,
                });
                at = to;
            }
            Step::Close => {
                close(&mut edges, at, start);
                at = start;
            }
        }
    }
    close(&mut edges, at, start);
    if !edges.is_empty() {
        subpaths.push(edges);
    }
    subpaths
}
