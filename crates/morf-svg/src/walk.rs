//! Walking the document for everything that has an edge.

use morf_outline::Step;
use resvg::usvg;

use crate::{rewind, steps_of};

/// Every path under a group, in paint order, already placed.
///
/// `abs_transform` is the whole chain of transforms above a path, so applying
/// it puts every shape in the document's own coordinates and the nesting stops
/// mattering. Groups are walked for their children and nothing else: a group's
/// opacity, blend mode and clip say how a *picture* of it is composited, and
/// none of that survives into an outline. What a clip path would mean here is a
/// real question — it is a shape subtracted from a shape, which a field does
/// natively — but reading one as though it were not there is the wrong answer,
/// so a clipped group is left to say so rather than quietly drawn whole.
pub(crate) fn group(group: &usvg::Group, into: &mut Vec<Step>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(child) => self::group(child, into),
            usvg::Node::Path(path) => self::path(path, into),
            // An image is pixels, and text has been converted to paths by the
            // parser when a font was available. Neither leaves an outline here.
            usvg::Node::Image(_) | usvg::Node::Text(_) => {}
        }
    }
}

fn path(path: &usvg::Path, into: &mut Vec<Step>) {
    if !path.is_visible() {
        return;
    }
    let placed = path.data().clone().transform(path.abs_transform());
    let Some(placed) = placed else {
        return;
    };

    if let Some(fill) = path.fill() {
        let mut filled = Vec::new();
        steps_of(&placed, &mut filled);
        // A field counts windings, which is exactly what `nonzero` means. It is
        // not what `evenodd` means, so an even-odd path is re-wound until the
        // two agree — every loop nested an odd number deep is turned around,
        // which is the same rule stated the other way round.
        if fill.rule() == usvg::FillRule::EvenOdd {
            filled = rewind::by_nesting(&filled);
        }
        into.extend(filled);
    }

    // A line icon is all stroke and no fill, so the shape on screen *is* the
    // stroke. Widening it into its own outline is the only way it exists as a
    // shape at all — a field has no pen.
    if let Some(stroke) = path.stroke() {
        let widened =
            resvg::tiny_skia::PathStroker::new().stroke(&placed, &stroke.to_tiny_skia(), 1.0);
        if let Some(widened) = widened {
            steps_of(&widened, into);
        }
    }
}
