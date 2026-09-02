// Diagnostics that measure the pipeline rather than assert about it.
//
// Ignored by default: they write pictures and print tables, which is what a
// question like "why does large text look wrong" needs and what a test suite
// does not. Each of them exists because a guess about this pipeline turned out
// to be wrong and only a number settled it.

use crate::TextSystem;

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

/// Gate A: how far the resampled contours stray from the outline they came
/// from, per glyph, in output pixels at a headline size.
///
/// The resample is what the morph frames and the glyph-as-shape path are both
/// built from, so this error lands on screen directly. Spaced evenly with no
/// regard for corners, `W` strayed nine pixels at this size while `o` strayed
/// none — a curve survives being resampled and a corner does not.
#[test]
#[ignore]
fn probe_resample_hausdorff() {
    const DRAWN: f32 = 340.0;
    const FAMILY: &str = "sans-serif";
    let reference = crate::glyph_fields::field_reference_for(DRAWN);
    let mut text = TextSystem::new();
    let mut worst: Vec<(char, f32)> = Vec::new();
    for glyph in " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ\
[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"
        .chars()
    {
        if let Some(error) = text.probe_resample_error(glyph, reference, FAMILY) {
            // Glyph units are reference pixels; report what lands on screen.
            worst.push((glyph, error * DRAWN / reference));
        }
    }
    for probe in ['#', 'W', 'A', 'm', '8', 'o', 'E', 'k', 'z'] {
        if let Some((_, error)) = worst.iter().find(|entry| entry.0 == probe) {
            println!("  WATCH {probe:?} {error:.2} px");
        }
    }
    worst.sort_by(|a, b| b.1.total_cmp(&a.1));
    println!("reference {reference}, drawn {DRAWN}");
    for (glyph, error) in worst.iter().take(12) {
        println!("  {glyph:?}  {error:.2} px");
    }
    let peak = worst.first().map_or(0.0, |entry| entry.1);
    println!("worst of {} glyphs: {peak:.2} px", worst.len());
}

/// Gate B: does replacing the reading near a corner with the two half-planes
/// that meet there actually remove the error, or only move it?
///
/// The whole bet is in this number. Bilinear reproduces an affine function
/// exactly and a half-plane's distance is affine, so a corner — the max or min
/// of two of them — should come back essentially free. That is the claim; this
/// measures it, over every printable character rather than one crop, because
/// the last attempt looked right on one crop.
#[test]
#[ignore]
fn probe_corner_cell_error() {
    let mut text = TextSystem::new();
    for reference in [128.0_f32, 256.0] {
        let mut plain_worst = 0.0_f32;
        let mut corrected_worst = 0.0_f32;
        let mut offenders: Vec<(char, f32, f32)> = Vec::new();
        for glyph in "!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ\
[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"
            .chars()
        {
            if let Some((plain, corrected)) =
                text.probe_corner_cells(glyph, reference, "sans-serif")
            {
                plain_worst = plain_worst.max(plain);
                corrected_worst = corrected_worst.max(corrected);
                offenders.push((glyph, plain, corrected));
            }
        }
        offenders.sort_by(|a, b| b.2.total_cmp(&a.2));
        println!(
            "reference {reference}: worst plain {plain_worst:.3}, corrected {corrected_worst:.3} glyph units"
        );
        for (glyph, plain, corrected) in offenders.iter().take(6) {
            println!("    {glyph:?}  {plain:.3} -> {corrected:.3}");
        }
    }
}

/// Debug: is the corner's own half-plane distance even right near the corner?
#[test]
#[ignore]
fn probe_corner_sanity() {
    let mut text = TextSystem::new();
    text.probe_corner_sanity('#', 256.0);
}

/// Both gates, across faces built in genuinely different ways.
///
/// Corner behaviour is a property of the typeface, not of the pipeline. A
/// grotesque meets its stems at right angles; a serif brackets them with a
/// curve that leaves tangentially, so the join is not a corner at all and a
/// half-plane standing in for it is describing a curve; a script has almost no
/// corners; a pixel face is nothing but corners. One font is not a measurement.
#[test]
#[ignore]
fn probe_fonts() {
    const DRAWN: f32 = 340.0;
    const REFERENCE: f32 = 256.0;
    const SAMPLE: &str = "#WAmoe8B$4RQg";

    let families = [
        ("grotesque", "Roboto"),
        ("serif", "Liberation Serif"),
        ("mono", "Iosevka"),
        ("script", "Grape Nuts"),
        ("pixel", "basis33"),
        ("segmented", "Digital-7"),
    ];

    println!(
        "{:<11} {:>10} {:>10} {:>10} {:>10}",
        "face", "resample", "snapped", "plain", "corrected"
    );
    for (label, family) in families {
        let mut text = TextSystem::new();
        if !text.has_family(family) {
            println!("{label:<11}  (not installed: {family})");
            continue;
        }
        let mut resample_worst = 0.0_f32;
        let mut plain_worst = 0.0_f32;
        let mut corrected_worst = 0.0_f32;
        let mut seen = 0;
        for glyph in SAMPLE.chars() {
            if let Some(error) = text.probe_resample_error(glyph, REFERENCE, family) {
                resample_worst = resample_worst.max(error * DRAWN / REFERENCE);
                seen += 1;
            }
            if let Some((plain, corrected)) = text.probe_corner_cells(glyph, REFERENCE, family) {
                plain_worst = plain_worst.max(plain);
                corrected_worst = corrected_worst.max(corrected);
            }
        }
        if seen == 0 {
            println!("{label:<11}  (no outlines)");
            continue;
        }
        println!(
            "{label:<11} {:>10} {:>9.2} {:>10.3} {:>10.3}",
            "-", resample_worst, plain_worst, corrected_worst
        );
    }
}

/// The accelerated field generator must agree with the plain one exactly.
///
/// It is the shipping generator: every glyph on screen comes out of it. A
/// speed-up that changes a single byte is not a speed-up, it is a new
/// renderer — so this checks byte for byte, over faces built differently
/// enough to exercise long thin strokes, tight counters and stray marks.
#[test]
#[ignore]
fn probe_field_generator_agrees() {
    const REFERENCE: f32 = 128.0;
    let spread = crate::glyph_fields::field_spread_for(REFERENCE);
    let mut checked = 0;
    let mut differing = 0;
    for family in ["Roboto", "Liberation Serif", "Grape Nuts", "basis33"] {
        let mut text = TextSystem::new();
        if !text.has_family(family) {
            continue;
        }
        for glyph in "#WAmoe8B$4RQg@ilj.,'\"".chars() {
            let Some(key) = text.probe_outline_key_in(glyph, REFERENCE, family) else { continue };
            let Some(commands) = text.probe_outline_commands(key) else { continue };
            let segments = crate::glyph_fields::flatten(&commands);
            let Some(area) = crate::glyph_fields::segment_box(&segments, spread) else { continue };
            let Some(fast) = crate::glyph_fields::field_from_segments(&segments, area, spread)
            else { continue };
            let Some(slow) = crate::glyph_fields::field_by_brute_force(&segments, area, spread)
            else { continue };
            checked += 1;
            assert_eq!(fast.width, slow.width, "{family} {glyph:?} width");
            assert_eq!(fast.height, slow.height, "{family} {glyph:?} height");
            let worst = fast
                .data
                .iter()
                .zip(slow.data.iter())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            if worst != 0 {
                differing += 1;
                println!("  {family} {glyph:?}: worst byte difference {worst}");
            }
        }
    }
    println!("{checked} glyphs checked, {differing} differing");
    assert_eq!(differing, 0, "accelerated generator must match byte for byte");
}

/// What the acceleration is worth, at the sizes where the stall was visible.
#[test]
#[ignore]
fn probe_field_generator_timing() {
    use std::time::Instant;
    let mut text = TextSystem::new();
    for reference in [64.0_f32, 128.0, 256.0] {
        let spread = crate::glyph_fields::field_spread_for(reference);
        let mut fast_total = 0.0_f64;
        let mut slow_total = 0.0_f64;
        let mut glyphs = 0;
        for glyph in "#WAmoe8B$4RQg@".chars() {
            let Some(key) = text.probe_outline_key_in(glyph, reference, "Roboto") else { continue };
            let Some(commands) = text.probe_outline_commands(key) else { continue };
            let segments = crate::glyph_fields::flatten(&commands);
            let Some(area) = crate::glyph_fields::segment_box(&segments, spread) else { continue };
            let mark = Instant::now();
            let _ = crate::glyph_fields::field_from_segments(&segments, area, spread);
            fast_total += mark.elapsed().as_secs_f64() * 1000.0;
            let mark = Instant::now();
            let _ = crate::glyph_fields::field_by_brute_force(&segments, area, spread);
            slow_total += mark.elapsed().as_secs_f64() * 1000.0;
            glyphs += 1;
        }
        let glyphs = glyphs.max(1) as f64;
        println!(
            "reference {reference:>5}: brute force {:>7.2} ms/glyph, accelerated {:>6.2} ms/glyph  ({:.1}x)",
            slow_total / glyphs,
            fast_total / glyphs,
            slow_total / fast_total.max(1e-9)
        );
    }
}
