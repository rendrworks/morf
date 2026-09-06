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
        style: morf_layout::TextStyle::default(),
        decoration: None,
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

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_decoration_is_a_band_under_the_line() {
    // A line under the text is drawn from the face's own metrics: below the
    // baseline, as wide as the line, in the decoration's colour — and not
    // there at all when nothing asked for it.
    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Text);
    let render = |decoration: Option<morf_scene::TextDecoration>| {
        let mut command = text_command(node, "mmmm", 40.0, DistanceFieldStyle::default());
        let DrawCommand::Text {
            decoration: slot, ..
        } = &mut command
        else {
            panic!("text_command builds text");
        };
        *slot = decoration;
        render_readback(
            &DrawList {
                commands: vec![command],
                layers: Vec::new(),
            },
            128,
        )
    };
    let plain = render(None);
    let underlined = render(Some(morf_scene::TextDecoration {
        line: morf_scene::DecorationLine::Under,
        thickness: Some(4.0),
        offset: 0.0,
        color: Some(Color::rgba8(255, 0, 0, 255)),
    }));
    // A row below the letters' baseline that the underline runs along:
    // the widest run of red across any row.
    let red_run = |pixels: &[u8], y: u32| {
        (0..128u32)
            .filter(|x| {
                let i = ((y * 128 + x) * 4) as usize;
                pixels[i] > 200 && pixels[i + 1] < 60 && pixels[i + 3] > 200
            })
            .count()
    };
    let widest = (0..128u32).map(|y| red_run(&underlined, y)).max().unwrap();
    assert!(widest > 60, "the line runs the width of the text: {widest}");
    assert_eq!(
        (0..128u32).map(|y| red_run(&plain, y)).max().unwrap(),
        0,
        "no red without a decoration"
    );
    assert!(
        ink(&underlined, 128) > ink(&plain, 128),
        "the band adds ink"
    );
}
