//! Line height, spacing, slant and the metrics a decoration is drawn from.

use morf_layout::{FontStretch, FontStyle, LineHeight, TextMeasurer, TextOptions, TextStyle};
use morf_scene::{Element, Scene};

use super::*;

#[test]
fn a_style_sets_the_line_height_and_spaces_the_letters() {
    // The line height is a multiple or a size; letter spacing widens a word
    // by a step per letter; word spacing widens it once per space and moves
    // the glyphs after the space along.
    let mut text = TextSystem::new();
    let mut scene = Scene::new();
    let node = scene.create(Element::Text);
    let plain = text.measure(node, "ab cd", "sans-serif", 16.0, TextOptions::default());
    assert!((plain.height - 16.0 * 1.2).abs() < 0.01, "{}", plain.height);

    let tall = text.measure(
        node,
        "ab cd",
        "sans-serif",
        16.0,
        TextOptions {
            style: TextStyle {
                line_height: LineHeight::Pixels(30.0),
                ..TextStyle::default()
            },
            ..TextOptions::default()
        },
    );
    assert!((tall.height - 30.0).abs() < 0.01, "{}", tall.height);
    let doubled = text.measure(
        node,
        "ab cd",
        "sans-serif",
        16.0,
        TextOptions {
            style: TextStyle {
                line_height: LineHeight::Multiple(2.0),
                ..TextStyle::default()
            },
            ..TextOptions::default()
        },
    );
    assert!((doubled.height - 32.0).abs() < 0.01, "{}", doubled.height);

    let tracked = text.measure(
        node,
        "ab cd",
        "sans-serif",
        16.0,
        TextOptions {
            style: TextStyle {
                letter_spacing: 4.0,
                ..TextStyle::default()
            },
            ..TextOptions::default()
        },
    );
    assert!(
        tracked.width > plain.width + 12.0,
        "tracking widened {} to {}",
        plain.width,
        tracked.width
    );

    let spaced = text.measure(
        node,
        "ab cd",
        "sans-serif",
        16.0,
        TextOptions {
            style: TextStyle {
                word_spacing: 10.0,
                ..TextStyle::default()
            },
            ..TextOptions::default()
        },
    );
    assert!(
        (spaced.width - plain.width - 10.0).abs() < 0.01,
        "one space, ten wider: {} to {}",
        plain.width,
        spaced.width
    );
    let span = |text: &mut TextSystem| {
        let glyphs = text.rasterize(node, (0.0, 0.0), 1.0, false);
        let first = glyphs.iter().map(|glyph| glyph.x).fold(f32::MAX, f32::min);
        let last = glyphs.iter().map(|glyph| glyph.x).fold(f32::MIN, f32::max);
        last - first
    };
    let spaced_span = span(&mut text);
    text.measure(node, "ab cd", "sans-serif", 16.0, TextOptions::default());
    let plain_span = span(&mut text);
    assert!(
        (spaced_span - plain_span - 10.0).abs() < 0.5,
        "the letters after the space moved along by the spacing: {plain_span} to {spaced_span}"
    );

    let italic = text.measure(
        node,
        "ab cd",
        "sans-serif",
        16.0,
        TextOptions {
            style: TextStyle {
                font_style: FontStyle::Italic,
                font_stretch: FontStretch::Condensed,
                ..TextStyle::default()
            },
            ..TextOptions::default()
        },
    );
    assert!(italic.width > 0.0);

    let bands = text.line_bands(node);
    assert_eq!(bands.len(), 1);
    assert!(bands[0].underline_offset > 0.0 && bands[0].stroke_size > 0.0);
    assert!((bands[0].baseline - 16.0 * 1.2 * 0.5).abs() < 16.0);
}
