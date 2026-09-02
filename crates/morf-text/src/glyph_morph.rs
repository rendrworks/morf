//! Turning one letter into another.
//!
//! The machinery is in `morf-outline`, because none of it is about letters: a
//! letter is broken into closed contours, the contours are matched, and each
//! pair is walked point by point. A drawing goes through the same functions,
//! which is why a letter can morph into an icon and not only into another
//! letter.
//!
//! What is here is the font side of it — the conversion into outline steps, and
//! the two entry points the glyph fields and the polygon layers ask for.

use cosmic_text::Command;
use morf_outline::Segment;

use crate::glyph_steps::steps;

pub use morf_outline::CONTOUR_POINTS;

/// Breaks a letter's outline into closed loops of evenly spaced points.
pub(crate) fn contours(commands: &[Command]) -> Vec<morf_outline::Contour> {
    morf_outline::contours(&steps(commands))
}

/// The outline partway between two letters, as straight pieces to measure.
pub(crate) fn between(paired: &[morf_outline::Paired], travel: f32) -> Vec<Segment> {
    morf_outline::between(paired, travel)
}

pub(crate) use morf_outline::{Contour, contour_points, pair_up, walk};
