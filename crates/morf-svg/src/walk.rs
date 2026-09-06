//! Walking the document for everything that has an edge.

use morf_outline::Step;
use resvg::usvg;

use crate::{clip, rewind, steps_of};

/// Why a document could not be turned into an outline.
pub(crate) struct Refused(pub(crate) String);

/// Every path under a group, in paint order, already placed.
///
/// `abs_transform` is the whole chain of transforms above a path, so applying
/// it puts every shape in the document's own coordinates and the nesting stops
/// mattering. A group's opacity, blend mode and mask say how a *picture* of it
/// is composited and none of that survives into an outline, so they are passed
/// over. A clip does survive — it is an intersection, and an intersection is a
/// shape — so it is taken here.
pub(crate) fn group(group: &usvg::Group, into: &mut Vec<Step>) -> Result<(), Refused> {
    for node in group.children() {
        match node {
            usvg::Node::Group(child) => {
                let Some(clip) = child.clip_path() else {
                    self::group(child, into)?;
                    continue;
                };
                let region = clip::region(clip)
                    .map_err(|_| Refused(format!("clip path `{}`", clip.id())))?;
                let mut clipped = Vec::new();
                self::group(child, &mut clipped)?;
                let kept = clip::apply(&clipped, &region)
                    .map_err(|_| Refused(format!("clip path `{}`", clip.id())))?;
                into.extend(kept);
            }
            usvg::Node::Path(path) => self::path(path, into),
            // An image is pixels, and text has been converted to paths by the
            // parser when a font was available. Neither leaves an outline here.
            usvg::Node::Image(_) | usvg::Node::Text(_) => {}
        }
    }
    Ok(())
}

/// The same walk with clips ignored, for gathering a clip's own shapes — where
/// a nested clip is handled by the region itself rather than here.
pub(crate) fn group_into(group: &usvg::Group, into: &mut Vec<Step>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(child) => group_into(child, into),
            usvg::Node::Path(path) => clip::shape_into(path, into),
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
