use cosmic_text::fontdb::Source;
use morf_scene::{Element, Scene};

use morf_layout::TextMeasurer;

use super::*;

#[test]
fn shapes_and_caches_a_buffer_per_text_node() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Text);
    let mut text = TextSystem::new();

    let measured = text.measure(node, "morf", "sans-serif", 16.0, TextOptions::default());

    assert!(measured.width > 0.0);
    assert!(measured.height > 0.0);
    assert!(text.buffer(node).is_some());
}

#[test]
fn font_weight_participates_in_the_shaping_cache() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Text);
    let mut text = TextSystem::new();

    text.measure(
        node,
        "morf",
        "sans-serif",
        16.0,
        TextOptions {
            font_weight: 700.0,
            ..TextOptions::default()
        },
    );

    assert_eq!(
        text.buffers[&crate::BufferKey::own(node)]
            .input
            .as_ref()
            .unwrap()
            .font_weight,
        700
    );
    assert_eq!(normalize_font_weight(50.0), 100);
    assert_eq!(normalize_font_weight(950.0), 900);
    assert_eq!(normalize_font_weight(f64::NAN), 400);
}

#[test]
fn missing_mono_family_keeps_monospace_advances() {
    let mut scene = Scene::new();
    let narrow = scene.create(Element::Text);
    let wide = scene.create(Element::Text);
    let mut text = TextSystem::new();

    let narrow = text.measure(
        narrow,
        "iiii",
        "Unavailable Nerd Font Mono",
        16.0,
        TextOptions::default(),
    );
    let wide = text.measure(
        wide,
        "WWWW",
        "Unavailable Nerd Font Mono",
        16.0,
        TextOptions::default(),
    );

    assert_eq!(
        text.resolved_family("Unavailable Nerd Font Mono"),
        "monospace"
    );
    assert!((narrow.width - wide.width).abs() < 0.01);
}

#[test]
fn family_stack_uses_an_installed_fallback() {
    let text = TextSystem::new();
    let installed = text
        .fonts
        .db()
        .faces()
        .find_map(|face| face.families.first())
        .map(|(family, _)| family.clone())
        .expect("system font database should not be empty");
    let request = format!("Missing Family, '{installed}'");

    assert_eq!(text.resolved_family(&request), installed);
    assert!(text.has_family(&installed.to_ascii_uppercase()));
}

#[test]
fn missing_font_path_reports_an_error() {
    let mut text = TextSystem::new();
    let error = text
        .load_font_path("/morf-test-font-does-not-exist.ttf")
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
fn text_font_source_loads_once_before_shaping() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Text);
    let mut text = TextSystem::new();
    let (source, family) = text
        .fonts
        .db()
        .faces()
        .find_map(|face| match &face.source {
            Source::File(path) => face
                .families
                .first()
                .map(|(family, _)| (path.clone(), family.clone())),
            _ => None,
        })
        .expect("system database should contain a file-backed font");
    let source = format!("file://{}", source.display());
    let before = text.fonts.db().len();
    let options = TextOptions {
        font_source: Some(source.clone()),
        ..TextOptions::default()
    };

    text.measure(node, "morf", &family, 16.0, options.clone());
    let loaded = text.fonts.db().len();
    text.measure(node, "morf", &family, 16.0, options);

    assert!(loaded > before);
    assert_eq!(text.fonts.db().len(), loaded);
    assert_eq!(text.font_sources.len(), 1);
    assert_eq!(
        text.buffers[&crate::BufferKey::own(node)]
            .input
            .as_ref()
            .unwrap()
            .font_source,
        Some(source)
    );
}

#[test]
fn wrapping_constrains_width_and_increases_height() {
    let mut scene = Scene::new();
    let unwrapped_node = scene.create(Element::Text);
    let wrapped_node = scene.create(Element::Text);
    let mut text = TextSystem::new();
    let content = "a shell runtime configured entirely in Lua";

    let unwrapped = text.measure(
        unwrapped_node,
        content,
        "sans-serif",
        16.0,
        TextOptions {
            width: Some(80.0),
            wrap: false,
            ..TextOptions::default()
        },
    );
    let wrapped = text.measure(
        wrapped_node,
        content,
        "sans-serif",
        16.0,
        TextOptions {
            width: Some(80.0),
            wrap: true,
            ..TextOptions::default()
        },
    );

    assert!(wrapped.width <= 80.0);
    assert!(wrapped.height > unwrapped.height);
    assert!(unwrapped.width > 80.0);
}

#[test]
fn centered_text_offsets_glyphs_inside_width() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Text);
    let mut text = TextSystem::new();
    text.measure(
        node,
        "morf",
        "sans-serif",
        16.0,
        TextOptions {
            width: Some(200.0),
            alignment: TextAlignment::Center,
            ..TextOptions::default()
        },
    );

    let first_x = text
        .buffer(node)
        .unwrap()
        .layout_runs()
        .next()
        .unwrap()
        .glyphs[0]
        .x;
    assert!(first_x > 0.0);
}

#[test]
fn rasterizes_cached_text_at_fractional_scale() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Text);
    let mut text = TextSystem::new();
    text.measure(node, "morf", "sans-serif", 16.0, TextOptions::default());

    let glyphs = text.rasterize(node, (5.0, 7.0), 1.25, false);
    let cached = text.rasterize(node, (5.0, 7.0), 1.25, false);

    assert!(!glyphs.is_empty());
    assert!(
        glyphs
            .iter()
            .all(|glyph| glyph.width > 0 && glyph.height > 0)
    );
    assert!(glyphs.iter().all(|glyph| !glyph.data.is_empty()));
    assert_eq!(
        glyphs
            .iter()
            .map(|glyph| glyph.cache_key)
            .collect::<Vec<_>>(),
        cached
            .iter()
            .map(|glyph| glyph.cache_key)
            .collect::<Vec<_>>()
    );
}

#[test]
fn eliding_places_ellipsis_and_constrains_width() {
    let mut fonts = FontSystem::new();
    let text = "application launcher settings";
    for mode in [TextElide::Left, TextElide::Middle, TextElide::Right] {
        let displayed = elided_text(
            &mut fonts,
            text,
            "sans-serif",
            16.0,
            &TextOptions {
                width: Some(100.0),
                elide: mode,
                ..TextOptions::default()
            },
        );
        assert!(displayed.contains('…'));
        assert!(shaped_width(&mut fonts, &displayed, "sans-serif", 16.0, 400.0) <= 100.0);
        match mode {
            TextElide::Left => assert!(text.ends_with(displayed.trim_start_matches('…'))),
            TextElide::Middle => {
                let (left, right) = displayed.split_once('…').unwrap();
                assert!(text.starts_with(left));
                assert!(text.ends_with(right));
            }
            TextElide::Right => assert!(text.starts_with(displayed.trim_end_matches('…'))),
            TextElide::None => unreachable!(),
        }
    }
}

#[test]
fn ordinary_text_is_rasterized_at_its_own_size() {
    // A distance field is measured once at a reference size and scaled, which
    // is right for a glyph being animated through sizes and wrong for a label.
    // Sixteen-pixel text built from a sixty-four-pixel field arrives with no
    // hinting and a soft edge — worse than what it replaced, at the size most
    // text is actually drawn.
    let mut scene = Scene::new();
    let node = scene.create(Element::Text);
    let mut text = TextSystem::new();
    text.measure(node, "morf", "sans-serif", 16.0, TextOptions::default());

    let direct = text.rasterize(node, (0.0, 0.0), 1.0, false);
    assert!(!direct.is_empty());
    assert!(
        direct
            .iter()
            .all(|glyph| glyph.content != RasterContent::Field),
        "no field where none was asked for",
    );
    // Drawn at the size it was measured, so the quad is the ink itself rather
    // than a scaled copy of a reference.
    assert!(
        direct
            .iter()
            .all(|glyph| glyph.draw_width == glyph.width as f32
                && glyph.draw_height == glyph.height as f32),
    );
}

#[test]
fn a_field_is_still_available_when_it_is_asked_for() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Text);
    let mut text = TextSystem::new();
    text.measure(node, "morf", "sans-serif", 16.0, TextOptions::default());

    let fields = text.rasterize(node, (0.0, 0.0), 1.0, true);
    assert!(!fields.is_empty());
    assert!(
        fields
            .iter()
            .any(|glyph| glyph.content == RasterContent::Field),
        "asking for a field gets one",
    );
}

/// A letter is one particular outline, and which one depends on the face.
#[test]
fn a_face_decides_which_outline_a_letter_is() {
    let mut text = TextSystem::new();
    let Some(other) = installed_face_unlike("serif", &mut text) else {
        return;
    };
    let serif = text.glyph_outline('8', None, 0.0, "serif", "serif");
    let elsewhere = text.glyph_outline('8', None, 0.0, &other, &other);
    assert!(!serif.is_empty() && !elsewhere.is_empty());
    assert_ne!(
        serif, elsewhere,
        "two faces are two outlines of the same `8`"
    );
}

/// Two faces morph into one another, because correspondence is geometry.
///
/// Nothing in the matching asks where an outline came from, so a letter walks
/// onto another face's letter the way it walks onto its own — which is what
/// makes changing the face an animation rather than a swap.
#[test]
fn a_letter_walks_onto_another_face() {
    let mut text = TextSystem::new();
    let Some(other) = installed_face_unlike("serif", &mut text) else {
        return;
    };
    let start = text.glyph_outline('W', Some('W'), 0.0, "serif", &other);
    let end = text.glyph_outline('W', Some('W'), 1.0, "serif", &other);
    let half = text.glyph_outline('W', Some('W'), 0.5, "serif", &other);
    assert_eq!(
        start.len(),
        end.len(),
        "one correspondence, one point count"
    );
    assert_eq!(half.len(), start.len());
    assert_ne!(start, end, "the two faces are not the same W");
    // Halfway is between the two rather than either of them: the letter is
    // travelling, not waiting to be replaced at the end.
    assert_ne!(half, start);
    assert_ne!(half, end);
}

/// The first installed face whose `8` differs from the named one's.
fn installed_face_unlike(from: &str, text: &mut TextSystem) -> Option<String> {
    let reference = text.glyph_outline('8', None, 0.0, from, from);
    ["monospace", "cursive", "fantasy", "sans-serif"]
        .into_iter()
        .find(|face| text.glyph_outline('8', None, 0.0, face, face) != reference)
        .map(str::to_owned)
}
