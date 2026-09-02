// Where a letter turns sharply, taken from the curves rather than from the
// pieces they were broken into.
//
// A corner has to be found before flattening, not after. Once an outline is
// chopped into straight pieces at a twentieth of a pixel, every join along a
// tight curve turns by a few degrees, and an angle test applied to those pieces
// calls all of them corners — which is how an `o` ends up with ninety-six of
// them. The curves know better: a Bézier's direction at each end comes from its
// control points, and two curves meeting either continue each other or do not.

use cosmic_text::Command;

/// How far two curves must turn away from each other to be a corner.
///
/// The cosine of about eight degrees. Well above what a curve's own control
/// points produce where it joins its neighbour smoothly, well below the
/// shallowest corner a letterform has.
const CORNER_COSINE: f32 = 0.99;

/// The corner vertices of one closed subpath, in the order they are walked.
pub(crate) fn corner_points(commands: &[Command]) -> Vec<Vec<(f32, f32)>> {
    let mut subpaths = Vec::new();
    for edges in split_subpaths(commands) {
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
/// boundary with a gap in it is not one. A `MoveTo` before an explicit `Close`
/// closes the previous subpath itself.
fn split_subpaths(commands: &[Command]) -> Vec<Vec<Edge>> {
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

    for command in commands {
        match command {
            Command::MoveTo(point) => {
                close(&mut edges, at, start);
                if !edges.is_empty() {
                    subpaths.push(std::mem::take(&mut edges));
                }
                start = (point.x, point.y);
                at = start;
            }
            Command::LineTo(point) => {
                let to = (point.x, point.y);
                let direction = normalise(to.0 - at.0, to.1 - at.1);
                edges.push(Edge {
                    start: at,
                    entering: direction,
                    leaving: direction,
                });
                at = to;
            }
            Command::QuadTo(control, point) => {
                let control = (control.x, control.y);
                let to = (point.x, point.y);
                let (entering, leaving) = quadratic_ends(at, control, to);
                edges.push(Edge {
                    start: at,
                    entering,
                    leaving,
                });
                at = to;
            }
            Command::CurveTo(first, second, point) => {
                let first = (first.x, first.y);
                let second = (second.x, second.y);
                let to = (point.x, point.y);
                let (entering, leaving) = cubic_ends(at, first, second, to);
                edges.push(Edge {
                    start: at,
                    entering,
                    leaving,
                });
                at = to;
            }
            Command::Close => {
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
