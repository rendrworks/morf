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

    assert_eq!(text.buffers[&node].input.as_ref().unwrap().font_weight, 700);
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
        text.buffers[&node].input.as_ref().unwrap().font_source,
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

    let glyphs = text.rasterize(node, (5.0, 7.0), 1.25);
    let cached = text.rasterize(node, (5.0, 7.0), 1.25);

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
