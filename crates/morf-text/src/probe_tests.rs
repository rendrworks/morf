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
    let reference = crate::glyph_fields::field_reference_for(DRAWN);
    let mut text = TextSystem::new();
    let mut worst: Vec<(char, f32)> = Vec::new();
    for glyph in " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ\
[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"
        .chars()
    {
        if let Some(error) = text.probe_resample_error(glyph, reference) {
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
