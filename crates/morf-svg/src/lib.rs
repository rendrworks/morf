//! An SVG, as an outline.
//!
//! Not as a picture of one. `resvg` can rasterise a document into pixels, and
//! for a photograph or a themed icon that is the right answer — but a shape
//! that has been rasterised has thrown away the thing a field is measured from.
//! Its edge is then only as exact as the grid it was flattened onto, it cannot
//! be scaled without being flattened again, and it cannot be walked onto
//! another shape at all, because there is nothing left to walk: a picture has
//! pixels, not points.
//!
//! So nothing here rasterises. The document is parsed, every filled path is
//! taken as the curves it was written as, every stroked path is turned into the
//! outline of its stroke, and the result is [`morf_outline::Step`]s — the same
//! thing a font hands over. From there an icon is a shape like any other: it
//! composes with a circle, it is cut out of a rectangle, and it morphs into a
//! letter, because by then nothing downstream can tell them apart.

use std::fmt;
use std::path::Path;

use morf_outline::Step;
use resvg::tiny_skia::PathSegment;
use resvg::usvg;

mod cache;
mod clip;
mod rewind;
mod walk;

pub use cache::SvgOutlines;

#[cfg(test)]
mod tests;

/// A document's outlines, and the box they were drawn in.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    /// Every closed loop in the document, one after another.
    pub steps: Vec<Step>,
    /// The viewport the coordinates are in, so a caller that cares where the
    /// shape sits inside its own canvas can say so.
    pub width: f32,
    pub height: f32,
}

impl Outline {
    /// Whether there is anything to measure.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

#[derive(Debug)]
pub enum SvgError {
    Read(std::io::Error),
    Parse(String),
    /// Parsed, but with nothing in it that has an outline.
    Empty,
    /// Understood, but not expressible as one outline. A clip is an
    /// intersection, and a concave or disjoint one needs general polygon
    /// intersection to take — which this does not do, and will not pretend to:
    /// a clip half applied comes out as the drawing whole, which is the one
    /// answer that is certainly wrong.
    Unsupported(String),
}

impl fmt::Display for SvgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "cannot read the SVG: {error}"),
            Self::Parse(message) => write!(formatter, "cannot parse the SVG: {message}"),
            Self::Empty => write!(formatter, "the SVG has no outlines in it"),
            Self::Unsupported(what) => {
                write!(formatter, "the SVG uses a {what} this cannot take exactly")
            }
        }
    }
}

impl std::error::Error for SvgError {}

/// Reads a document from disk and takes its outlines.
pub fn outline_of(path: impl AsRef<Path>) -> Result<Outline, SvgError> {
    let bytes = std::fs::read(path).map_err(SvgError::Read)?;
    outline_from_bytes(&bytes)
}

/// Takes the outlines of a document already in memory.
pub fn outline_from_bytes(bytes: &[u8]) -> Result<Outline, SvgError> {
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &options)
        .map_err(|error| SvgError::Parse(error.to_string()))?;
    let size = tree.size();
    let mut steps = Vec::new();
    walk::group(tree.root(), &mut steps).map_err(|refused| SvgError::Unsupported(refused.0))?;
    if steps.is_empty() {
        return Err(SvgError::Empty);
    }
    Ok(Outline {
        steps,
        width: size.width(),
        height: size.height(),
    })
}

/// One `tiny-skia` path, in the outline's own terms.
///
/// The two vocabularies are the same vocabulary, which is the point: a curve is
/// a curve whether a font or a document wrote it down.
fn steps_of(path: &resvg::tiny_skia::Path, into: &mut Vec<Step>) {
    for segment in path.segments() {
        into.push(match segment {
            PathSegment::MoveTo(point) => Step::Move(point.x, point.y),
            PathSegment::LineTo(point) => Step::Line(point.x, point.y),
            PathSegment::QuadTo(control, point) => {
                Step::Quad(control.x, control.y, point.x, point.y)
            }
            PathSegment::CubicTo(first, second, point) => {
                Step::Cubic(first.x, first.y, second.x, second.y, point.x, point.y)
            }
            PathSegment::Close => Step::Close,
        });
    }
}
