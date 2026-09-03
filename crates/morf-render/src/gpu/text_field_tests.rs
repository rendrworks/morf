use morf_layout::{Geometry, TextAlignment, TextElide, Transform2D};
use morf_scene::{Color, NodeHandle};

use crate::*;

use crate::gpu::field_tests::{alpha_at, render_readback};

/// A text command, sized and styled, with everything else left alone.
pub(crate) fn text_command(
    node: NodeHandle,
    text: &str,
    size: f64,
    field_style: DistanceFieldStyle,
) -> DrawCommand {
    DrawCommand::Text {
        morph_to: String::new(),
        morph_progress: 0.0,
        node,
        bounds: Geometry {
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 128.0,
        },
        transform: Transform2D::IDENTITY,
        clip: None,
        text: text.to_owned(),
        family: "sans-serif".to_owned(),
        font_source: String::new(),
        size,
        font_weight: 400.0,
        color: Color::rgba8(255, 255, 255, 255),
        color_overlay: Color::rgba8(0, 0, 0, 0),
        wrap: false,
        max_lines: 0,
        elide: TextElide::None,
        horizontal_alignment: TextAlignment::Left,
        vertical_alignment: VerticalAlignment::Top,
        field_style,
    }
}

/// How much ink a rendering has, as a fraction of the pixels it could cover.
pub(crate) fn ink(pixels: &[u8], size: u32) -> f64 {
    let covered = (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .filter(|(x, y)| alpha_at(pixels, size, *x, *y) > 128)
        .count();
    covered as f64 / f64::from(size * size)
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_glyph_drawn_larger_covers_more_without_being_rasterized_again() {
    // The reason glyphs are stored as fields at all. One atlas entry, measured
    // once at a reference size, and the letter is drawn at whatever size is
    // asked for by scaling the quad. If this failed by drawing the same number
    // of pixels at both sizes, the field would be being sampled as if it were a
    // fixed-size bitmap.
    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Text);
    let render = |size| {
        let list = DrawList {
            commands: vec![text_command(node, "M", size, DistanceFieldStyle::default())],
            layers: Vec::new(),
        };
        ink(&render_readback(&list, 128), 128)
    };

    let small = render(24.0);
    let large = render(72.0);
    assert!(small > 0.0, "the small letter drew something: {small}");
    assert!(
        large > small * 3.0,
        "three times the size covers far more ground: {large} against {small}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn thickness_moves_the_edge_and_an_outline_adds_a_band_around_it() {
    // What a threshold buys that a coverage bitmap cannot: the edge is a number
    // rather than a set of pixels, so weight is that number moved and an
    // outline is a second one further out. Both are animatable, and neither
    // re-renders the glyph.
    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Text);
    let render = |style| {
        let list = DrawList {
            commands: vec![text_command(node, "M", 64.0, style)],
            layers: Vec::new(),
        };
        ink(&render_readback(&list, 128), 128)
    };

    let plain = render(DistanceFieldStyle::default());
    let bolder = render(DistanceFieldStyle {
        thickness: 2.0,
        ..DistanceFieldStyle::default()
    });
    let thinner = render(DistanceFieldStyle {
        thickness: -2.0,
        ..DistanceFieldStyle::default()
    });
    assert!(
        bolder > plain && plain > thinner,
        "the edge moved both ways: {thinner} < {plain} < {bolder}"
    );

    let outlined = render(DistanceFieldStyle {
        outline_width: 3.0,
        outline_color: Color::rgba8(255, 0, 0, 255),
        ..DistanceFieldStyle::default()
    });
    assert!(
        outlined > plain,
        "the outline is a band outside the fill: {outlined} against {plain}"
    );
}
