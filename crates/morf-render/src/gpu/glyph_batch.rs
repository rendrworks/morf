use crate::effects::color_array;
use crate::{DrawCommand, DrawList, VerticalAlignment};
use morf_layout::{Geometry, TextMeasurer, TextOptions};
use morf_text::{RasterContent, TextSystem};

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
            horizontal_alignment,
            vertical_alignment,
            field_style,
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
            },
        );
        let spare_height = (bounds.height - measured.height).max(0.0);
        let vertical_offset = match vertical_alignment {
            VerticalAlignment::Top => 0.0,
            VerticalAlignment::Center => spare_height / 2.0,
            VerticalAlignment::Bottom => spare_height,
        };
        for glyph in text_system.rasterize(
            *node,
            (
                bounds.x as f32 * scale,
                (bounds.y + vertical_offset) as f32 * scale,
            ),
            scale,
        ) {
            if glyph.width > 0 && glyph.height > 0 {
                glyphs.push(PreparedGlyph {
                    glyph,
                    color: *color,
                    color_overlay: *color_overlay,
                    transform: *transform,
                    command_index,
                    field: glyph_field_uniform(*field_style, *size),
                    outline_color: color_array(field_style.outline_color),
                });
            }
        }
    }
    if glyphs.is_empty() {
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
            field: prepared.field,
            outline_color: prepared.outline_color,
            ..GlyphInstance::default()
        });
    }
    Ok(Some(GlyphBatch {
        instances,
        command_spans,
    }))
}
