//! What text becomes when it does not fit.
//!
//! Split from the text system at the line gate. Two cases: an unwrapped
//! line that overflows its width takes an ellipsis at one end or in the
//! middle; wrapped text with a `max_lines` keeps that many lines and ends
//! the last with one. Both search over graphemes, each probe a throwaway
//! shape, so the cost is paid only by text that overflows.

use cosmic_text::{Align, Buffer, FontSystem, Shaping, Wrap};
use morf_layout::{TextElide, TextOptions};
use unicode_segmentation::UnicodeSegmentation;

use crate::style::{run_gain, text_attrs, text_metrics};
use crate::{normalize_font_weight, resolve_family};

pub(crate) fn elided_text(
    fonts: &mut FontSystem,
    text: &str,
    family: &str,
    size: f32,
    options: &TextOptions,
) -> String {
    if options.wrap
        && options.max_lines > 0
        && let Some(width) = options.width
    {
        return truncated_to_lines(fonts, text, family, size, options, width as f32);
    }
    let Some(width) = options
        .width
        .filter(|_| !options.wrap && options.elide != TextElide::None)
    else {
        return text.to_owned();
    };
    if shaped_width(fonts, text, family, size, options) <= width as f32 {
        return text.to_owned();
    }
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut low = 0;
    let mut high = graphemes.len();
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate = elide_candidate(&graphemes, middle, options.elide);
        if shaped_width(fonts, &candidate, family, size, options) <= width as f32 {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    elide_candidate(&graphemes, low, options.elide)
}

/// The longest prefix of `text` that wraps into `max_lines` lines at
/// `width`, with an ellipsis on the last, when the whole does not fit.
///
/// A binary search over graphemes, each probe a throwaway shape: the same
/// cost shape as single-line eliding, paid only by text that overflows.
fn truncated_to_lines(
    fonts: &mut FontSystem,
    text: &str,
    family: &str,
    size: f32,
    options: &TextOptions,
    width: f32,
) -> String {
    let lines = |fonts: &mut FontSystem, candidate: &str| {
        wrapped_lines(fonts, candidate, family, size, options, width)
    };
    if lines(fonts, text) <= options.max_lines {
        return text.to_owned();
    }
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut low = 0;
    let mut high = graphemes.len();
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate = elide_candidate(&graphemes, middle, TextElide::Right);
        if lines(fonts, &candidate) <= options.max_lines {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    elide_candidate(&graphemes, low, TextElide::Right)
}

fn wrapped_lines(
    fonts: &mut FontSystem,
    text: &str,
    family: &str,
    size: f32,
    options: &TextOptions,
    width: f32,
) -> usize {
    let family = resolve_family(fonts, family);
    let mut buffer = Buffer::new(fonts, text_metrics(size, &options.style));
    buffer.set_size(Some(width), None);
    buffer.set_wrap(Wrap::WordOrGlyph);
    buffer.set_text(
        text,
        &text_attrs(
            &family,
            normalize_font_weight(options.font_weight),
            size,
            &options.style,
        ),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(fonts, false);
    buffer.layout_runs().count()
}

fn elide_candidate(graphemes: &[&str], kept: usize, mode: TextElide) -> String {
    let kept = kept.min(graphemes.len());
    match mode {
        TextElide::None => graphemes.concat(),
        TextElide::Left => format!("…{}", graphemes[graphemes.len() - kept..].concat()),
        TextElide::Right => format!("{}…", graphemes[..kept].concat()),
        TextElide::Middle => {
            let left = kept.div_ceil(2);
            let right = kept - left;
            format!(
                "{}…{}",
                graphemes[..left].concat(),
                graphemes[graphemes.len() - right..].concat()
            )
        }
    }
}

pub(crate) fn shaped_width(
    fonts: &mut FontSystem,
    text: &str,
    family: &str,
    size: f32,
    options: &TextOptions,
) -> f32 {
    let family = resolve_family(fonts, family);
    let mut buffer = Buffer::new(fonts, text_metrics(size, &options.style));
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        text,
        &text_attrs(
            &family,
            normalize_font_weight(options.font_weight),
            size,
            &options.style,
        ),
        Shaping::Advanced,
        Some(Align::Left),
    );
    buffer.shape_until_scroll(fonts, false);
    let word_spacing = options.style.word_spacing as f32;
    buffer
        .layout_runs()
        .map(|run| run.line_w + run_gain(&run, word_spacing))
        .fold(0.0, f32::max)
}
