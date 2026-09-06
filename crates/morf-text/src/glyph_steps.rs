//! A font's outline, as an outline.
//!
//! `cosmic_text` hands back move/line/quadratic/cubic/close, which is what
//! `morf_outline::Step` is. This is the whole of what makes a letter and a
//! drawing the same kind of thing to everything downstream — the contours, the
//! resampling, the correspondence and the walk are one implementation, and a
//! font is simply one of the things that can be poured into it.

use cosmic_text::Command;
use morf_outline::Step;

pub(crate) fn steps(commands: &[Command]) -> Vec<Step> {
    commands
        .iter()
        .map(|command| match command {
            Command::MoveTo(point) => Step::Move(point.x, point.y),
            Command::LineTo(point) => Step::Line(point.x, point.y),
            Command::QuadTo(control, point) => Step::Quad(control.x, control.y, point.x, point.y),
            Command::CurveTo(first, second, point) => {
                Step::Cubic(first.x, first.y, second.x, second.y, point.x, point.y)
            }
            Command::Close => Step::Close,
        })
        .collect()
}
