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

/// Diagnostic: writes the same glyph twice, as the traced outline and as the
/// field reconstructs it, so the two can be compared directly.
#[test]
#[ignore]
fn probe_outline_against_field() {
    use crate::glyph_fields::{field_reference_for, field_spread_for, flatten, glyph_field};

    const DRAWN: usize = 340;
    let glyph = std::env::var("PROBE_GLYPH")
        .ok()
        .and_then(|text| text.chars().next())
        .unwrap_or('#');

    let reference = field_reference_for(DRAWN as f32);
    let mut text = TextSystem::new();
    let key = text
        .probe_outline_key(glyph, reference)
        .expect("glyph shapes");
    let commands = text
        .probe_outline_commands(key)
        .expect("glyph has an outline");
    let spread = field_spread_for(reference);
    let segments = flatten(&commands);

    // Where the outline sits, so both pictures frame it the same way.
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for piece in &segments {
        min_x = min_x.min(piece.x0).min(piece.x1);
        max_x = max_x.max(piece.x0).max(piece.x1);
        min_y = min_y.min(piece.y0).min(piece.y1);
        max_y = max_y.max(piece.y0).max(piece.y1);
    }
    let scale = (DRAWN as f32 * 0.8) / (max_x - min_x).max(max_y - min_y);
    let pad = DRAWN as f32 * 0.1;
    let to_glyph = |px: f32, py: f32| (min_x + (px - pad) / scale, max_y - (py - pad) / scale);

    // The outline itself, filled by winding at sixteen samples a pixel. This is
    // the trace, with nothing between it and the picture.
    let mut traced = vec![0u8; DRAWN * DRAWN];
    for row in 0..DRAWN {
        for column in 0..DRAWN {
            let mut covered = 0;
            for sub in 0..16 {
                let (ox, oy) = (
                    (sub % 4) as f32 * 0.25 + 0.125,
                    (sub / 4) as f32 * 0.25 + 0.125,
                );
                let (gx, gy) = to_glyph(column as f32 + ox, row as f32 + oy);
                let winding: i32 = segments.iter().map(|piece| piece.winding(gx, gy)).sum();
                covered += i32::from(winding != 0);
            }
            traced[row * DRAWN + column] = (covered * 255 / 16) as u8;
        }
    }

    // The same glyph through the field: measured at the reference size, then
    // sampled back the way the shader samples it.
    let field = glyph_field(&commands, spread).expect("glyph has a field");
    println!(
        "reference {reference}  spread {spread}  scale {scale:.2}\n\
         ink {:.1}x{:.1} glyph units   field {}x{} texels",
        max_x - min_x,
        max_y - min_y,
        field.width,
        field.height
    );
    let sample = |x: f32, y: f32| -> f32 {
        // Texel `i` holds the field at `left + i + 0.5`, so reading position
        // `x` means interpolating around `x - left - 0.5`. The GPU's own
        // convention already does this; the probe has to do it by hand.
        let fx = (x - field.left - 0.5).clamp(0.0, field.width as f32 - 1.001);
        let fy = (field.top - y - 0.5).clamp(0.0, field.height as f32 - 1.001);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |cx: usize, cy: usize| {
            f32::from(
                field.data[cy.min(field.height as usize - 1) * field.width as usize
                    + cx.min(field.width as usize - 1)],
            ) / 255.0
        };
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
        let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    };
    let ramp = crate::field_units_per_logical_px(DRAWN as f32) / scale * 0.0
        + 0.5 / (spread * 2.0 * scale);
    let mut fielded = vec![0u8; DRAWN * DRAWN];
    for row in 0..DRAWN {
        for column in 0..DRAWN {
            let (gx, gy) = to_glyph(column as f32 + 0.5, row as f32 + 0.5);
            let value = sample(gx, gy);
            let coverage = ((0.5 - value) / ramp.max(1e-6) + 0.5).clamp(0.0, 1.0);
            fielded[row * DRAWN + column] = (coverage * 255.0) as u8;
        }
    }

    // The same distance, computed at every output pixel instead of stored at
    // the reference and read back. If this is sharp and `fielded` is not, the
    // loss is the storing; if both are round, it is the measuring.
    let mut analytic = vec![0u8; DRAWN * DRAWN];
    for row in 0..DRAWN {
        for column in 0..DRAWN {
            let (gx, gy) = to_glyph(column as f32 + 0.5, row as f32 + 0.5);
            let mut nearest = f32::MAX;
            let mut winding = 0;
            for piece in &segments {
                nearest = nearest.min(piece.distance_squared(gx, gy));
                winding += piece.winding(gx, gy);
            }
            let signed = if winding != 0 {
                -nearest.sqrt()
            } else {
                nearest.sqrt()
            };
            // In glyph units, faded over one output pixel.
            let coverage = (0.5 - signed * scale / 1.0).clamp(0.0, 1.0);
            analytic[row * DRAWN + column] = (coverage * 255.0) as u8;
        }
    }

    for (name, pixels) in [
        ("traced", &traced),
        ("fielded", &fielded),
        ("analytic", &analytic),
    ] {
        let mut out = format!("P5\n{DRAWN} {DRAWN}\n255\n").into_bytes();
        out.extend_from_slice(pixels);
        std::fs::write(format!("/tmp/glyph_{name}.pgm"), out).expect("write probe");
    }
    // What the stored field says along a row crossing a corner, against what
    // the outline says there. If the two agree, the loss is in the reading.
    let probe_row = DRAWN / 2;
    let mut worst = 0.0_f32;
    for column in 0..DRAWN {
        let (gx, gy) = to_glyph(column as f32 + 0.5, probe_row as f32 + 0.5);
        let stored = (sample(gx, gy) * 2.0 - 1.0) * spread;
        let mut nearest = f32::MAX;
        let mut winding = 0;
        for piece in &segments {
            nearest = nearest.min(piece.distance_squared(gx, gy));
            winding += piece.winding(gx, gy);
        }
        let truth = if winding != 0 {
            -nearest.sqrt()
        } else {
            nearest.sqrt()
        };
        if truth.abs() < spread {
            worst = worst.max((stored - truth).abs());
        }
    }
    println!("worst stored-vs-true distance error inside the spread: {worst:.3} glyph units");
    println!(
        "one texel is 1.0 glyph unit; one output pixel is {:.3}",
        1.0 / scale
    );
    println!("wrote /tmp/glyph_traced.pgm and /tmp/glyph_fielded.pgm");
}
