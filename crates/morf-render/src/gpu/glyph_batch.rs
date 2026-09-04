use crate::effects::color_array;
use crate::{DrawCommand, DrawList, VerticalAlignment};
use morf_layout::{Geometry, TextMeasurer, TextOptions, Transform2D};
use morf_scene::{Color, DecorationLine, TextDecoration};
use morf_text::{LineBand, RasterContent, RasterGlyph, TextSystem};

use super::{backend_types::*, glyphs::*, textures::*};

// Shaped text on its way to the screen: one batch of glyph quads per frame,
// drawn from the atlas the glyphs were measured into.

pub(crate) struct GlyphBatchContext<'a> {
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) mask_atlas: &'a mut GlyphAtlas,
    pub(crate) color_atlas: &'a mut GlyphAtlas,
    pub(crate) target_size: (u32, u32),
}

pub(crate) fn create_glyph_batch(
    context: GlyphBatchContext<'_>,
    text_system: &mut TextSystem,
    list: &DrawList,
    scale_120: u32,
) -> Result<Option<GlyphBatch>, GpuError> {
    let GlyphBatchContext {
        queue,
        mask_atlas,
        color_atlas,
        target_size: (target_width, target_height),
    } = context;
    let scale = scale_120.max(1) as f32 / 120.0;
    let mut glyphs = Vec::new();
    let mut bands: Vec<PreparedBand> = Vec::new();
    for (command_index, command) in list.commands.iter().enumerate() {
        let DrawCommand::Text {
            node,
            bounds,
            transform,
            text,
            family,
            font_source,
            size,
            font_weight,
            color,
            color_overlay,
            wrap,
            elide,
            max_lines,
            horizontal_alignment,
            vertical_alignment,
            field_style,
            morph_to,
            morph_progress,
            style,
            decoration,
            ..
        } = command
        else {
            continue;
        };
        let measured = text_system.measure(
            *node,
            text,
            family,
            *size,
            TextOptions {
                width: Some(bounds.width),
                wrap: *wrap,
                alignment: *horizontal_alignment,
                elide: *elide,
                font_weight: *font_weight,
                font_source: (!font_source.is_empty()).then(|| font_source.clone()),
                max_lines: *max_lines,
                style: *style,
            },
        );
        let spare_height = (bounds.height - measured.height).max(0.0);
        let vertical_offset = match vertical_alignment {
            VerticalAlignment::Top => 0.0,
            VerticalAlignment::Center => spare_height / 2.0,
            VerticalAlignment::Bottom => spare_height,
        };
        // Every glyph that has an outline is drawn from its field. There used
        // to be a size below which a direct rasterization won, and it won for a
        // reason that has since been removed: the field was measured at one
        // size for all text, so small text read a sixty-four pixel field
        // through eleven pixels and came back scarred by the minification. A
        // field is now measured at a reference chosen from the size it will be
        // drawn at, so it is never read at worse than half its own resolution.
        //
        // What that buys is one representation for all text: a size can be
        // animated without refilling the atlas, and thickness, softness and an
        // outline are thresholds every label can ask for rather than a
        // privilege of large ones.
        let morphing = *morph_progress > 0.0 && !morph_to.is_empty();
        let origin = (
            bounds.x as f32 * scale,
            (bounds.y + vertical_offset) as f32 * scale,
        );
        // How much the field changes across one device pixel, from the size the
        // glyph is drawn at, so the edge can fade over exactly one pixel
        // without measuring anything.
        let ramp = morf_text::field_units_per_logical_px(*size as f32) / scale.max(f32::EPSILON);
        let mut push = |glyph: RasterGlyph, morph: Option<RasterGlyph>, progress: f32| {
            if glyph.width > 0 && glyph.height > 0 {
                glyphs.push(PreparedGlyph {
                    glyph,
                    morph,
                    morph_progress: progress,
                    ramp,
                    color: *color,
                    color_overlay: *color_overlay,
                    transform: *transform,
                    command_index,
                    field: glyph_field_uniform(*field_style, *size),
                    outline_color: color_array(field_style.outline_color),
                });
            }
        };

        if morphing {
            text_system.measure_target(
                *node,
                morph_to,
                family,
                *size,
                TextOptions {
                    width: Some(bounds.width),
                    wrap: *wrap,
                    alignment: *horizontal_alignment,
                    elide: *elide,
                    font_weight: *font_weight,
                    font_source: (!font_source.is_empty()).then(|| font_source.clone()),
                    max_lines: *max_lines,
                    style: *style,
                },
            );
            // Paired glyphs come back already measured over one shared box, so
            // both are read through the same quad and the same coordinates —
            // there is nothing left here to reconcile between them.
            // The travel is resolved into a pair of neighbouring frames and a
            // local position between them, so what reaches the shader is always
            // a short step.
            for (glyph, partner, local) in
                text_system.rasterize_pairs(*node, origin, scale, *morph_progress)
            {
                push(glyph, partner, local);
            }
            // Whatever the target has that the source does not is arriving
            // rather than leaving, so it runs the same interpolation backwards:
            // dissolved at zero and whole at one.
            let own = text_system.rasterize(*node, origin, scale, true).len();
            for glyph in text_system
                .rasterize_target(*node, origin, scale, true)
                .into_iter()
                .skip(own)
            {
                push(glyph, None, 1.0 - *morph_progress);
            }
        } else {
            for glyph in text_system.rasterize(*node, origin, scale, true) {
                push(glyph, None, 0.0);
            }
        }
        if let Some(decoration) = decoration {
            bands.extend(decoration_bands(
                text_system.line_bands(*node),
                decoration,
                *size,
                Geometry {
                    x: bounds.x,
                    y: bounds.y + vertical_offset,
                    width: bounds.width,
                    height: bounds.height,
                },
                *color,
                *color_overlay,
                *transform,
                command_index,
            ));
        }
    }
    if glyphs.is_empty() && bands.is_empty() {
        return Ok(None);
    }
    mask_atlas.prepare(queue, &glyphs)?;
    color_atlas.prepare(queue, &glyphs)?;
    let mut instances = Vec::with_capacity(glyphs.len());
    let mut command_spans: Vec<Vec<GlyphSpan>> =
        (0..list.commands.len()).map(|_| Vec::new()).collect();
    for prepared in glyphs {
        let glyph = prepared.glyph;
        let key = GlyphKey::from_glyph(&glyph);
        let color_glyph = glyph.content == RasterContent::Color;
        let atlas = if color_glyph {
            &*color_atlas
        } else {
            &*mask_atlas
        };
        let entry = atlas.entries.get(&key).ok_or_else(|| {
            GpuError("prepared glyph is missing from the persistent atlas".to_owned())
        })?;
        let tint = match glyph.content {
            RasterContent::Mask | RasterContent::Field => color_array(prepared.color),
            RasterContent::Color => [1.0, 1.0, 1.0, prepared.color.alpha],
        };
        let (origin, axes) = transformed_quad(
            prepared.transform,
            Geometry {
                x: f64::from(glyph.x) / f64::from(scale),
                y: f64::from(glyph.y) / f64::from(scale),
                // The quad, not the bitmap: a distance field is measured once
                // and drawn at any size, so these are the only two of the four
                // that follow the font size rather than the atlas.
                width: f64::from(glyph.draw_width) / f64::from(scale),
                height: f64::from(glyph.draw_height) / f64::from(scale),
            },
            f64::from(scale),
            (target_width, target_height),
        );
        // Where the partner sits in the atlas, or nothing — which the shader
        // reads as empty space so an unpaired letter dissolves.
        let morph_uv = prepared
            .morph
            .as_ref()
            .and_then(|partner| {
                let entry = atlas.entries.get(&GlyphKey::from_glyph(partner))?;
                Some([
                    entry.x as f32 / GLYPH_ATLAS_SIZE as f32,
                    entry.y as f32 / GLYPH_ATLAS_SIZE as f32,
                    partner.width as f32 / GLYPH_ATLAS_SIZE as f32,
                    partner.height as f32 / GLYPH_ATLAS_SIZE as f32,
                ])
            })
            .unwrap_or_default();
        let instance = instances.len() as u32;
        let spans = &mut command_spans[prepared.command_index];
        if let Some(span) = spans.last_mut()
            && span.color == color_glyph
            && span.range.end == instance
        {
            span.range.end = instance + 1;
        } else {
            spans.push(GlyphSpan {
                range: instance..instance + 1,
                color: color_glyph,
            });
        }
        instances.push(GlyphInstance {
            origin,
            axes,
            uv: [
                entry.x as f32 / GLYPH_ATLAS_SIZE as f32,
                entry.y as f32 / GLYPH_ATLAS_SIZE as f32,
                glyph.width as f32 / GLYPH_ATLAS_SIZE as f32,
                glyph.height as f32 / GLYPH_ATLAS_SIZE as f32,
            ],
            color: tint,
            color_overlay: color_array(prepared.color_overlay),
            mode: [
                0.0,
                0.0,
                if color_glyph { 0.0 } else { 1.0 },
                f32::from(glyph.content == RasterContent::Field),
            ],
            field: [
                prepared.field[0],
                prepared.field[1],
                prepared.field[2],
                prepared.morph_progress,
            ],
            outline_color: prepared.outline_color,
            morph_uv,
            ramp: prepared.ramp,
            ..GlyphInstance::default()
        });
    }
    // A decoration is a solid quad through the glyph pipeline, after the
    // letters of every command so it lies over them, clipped and masked the
    // same way.
    for band in bands {
        let (origin, axes) = transformed_quad(
            band.transform,
            band.rect,
            f64::from(scale),
            (target_width, target_height),
        );
        let instance = instances.len() as u32;
        command_spans[band.command_index].push(GlyphSpan {
            range: instance..instance + 1,
            color: false,
        });
        instances.push(GlyphInstance {
            origin,
            axes,
            color: color_array(band.color),
            color_overlay: color_array(band.color_overlay),
            // `z` past one is solid: no sample, the colour as it is.
            mode: [0.0, 0.0, 2.0, 0.0],
            ..GlyphInstance::default()
        });
    }
    Ok(Some(GlyphBatch {
        instances,
        command_spans,
    }))
}

/// One decoration line, positioned, waiting to become an instance.
struct PreparedBand {
    rect: Geometry,
    color: Color,
    color_overlay: Color,
    transform: Transform2D,
    command_index: usize,
}

/// Where a decoration runs along each line, from the face's own metrics.
#[allow(clippy::too_many_arguments)]
fn decoration_bands(
    lines: Vec<LineBand>,
    decoration: &TextDecoration,
    size: f64,
    bounds: Geometry,
    text_color: Color,
    color_overlay: Color,
    transform: Transform2D,
    command_index: usize,
) -> Vec<PreparedBand> {
    let color = decoration.color.unwrap_or(text_color);
    lines
        .into_iter()
        .map(|line| {
            let thickness = decoration
                .thickness
                .unwrap_or(f64::from(line.stroke_size))
                .max(size / 24.0);
            let baseline = bounds.y + f64::from(line.baseline);
            // Where the face puts the line, then the configuration's offset,
            // downwards; the band's own thickness is centred on that.
            let centre = match decoration.line {
                DecorationLine::Under => {
                    baseline + f64::from(line.underline_offset) + thickness / 2.0
                }
                DecorationLine::Over => baseline - f64::from(line.ascent) + thickness / 2.0,
                DecorationLine::Through => baseline - f64::from(line.strikeout_offset),
            } + decoration.offset;
            PreparedBand {
                rect: Geometry {
                    x: bounds.x + f64::from(line.x),
                    y: centre - thickness / 2.0,
                    width: f64::from(line.width),
                    height: thickness,
                },
                color,
                color_overlay,
                transform,
                command_index,
            }
        })
        .collect()
}
