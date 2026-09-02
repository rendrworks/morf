use crate::effects::color_array;
use crate::{DrawCommand, DrawList, VerticalAlignment};
use morf_layout::{Geometry, TextMeasurer, TextOptions};
use morf_text::{GLYPH_FIELD_REFERENCE_PX, RasterContent, RasterGlyph, TextSystem};

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
            morph_to,
            morph_progress,
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
        // A distance field is wanted only when it buys something. A style that
        // moves the edge, feathers it or draws an outline needs one; so does a
        // glyph drawn at or above the size the field was measured at, where a
        // direct rasterization would want its own cache entry per size.
        //
        // Below that it is a straight loss: an eleven-pixel label built from a
        // sixty-four-pixel field has no hinting and a soft edge, which is how
        // every label in the shell got worse when fields were switched on for
        // all text rather than for the text that asked.
        let styled = field_style.thickness != 0.0
            || field_style.softness != 0.0
            || field_style.outline_width != 0.0;
        let large = (*size as f32) * scale >= GLYPH_FIELD_REFERENCE_PX;
        // A morph needs fields on both sides. Two coverage bitmaps average into
        // a double exposure of two letters; two distance fields average into a
        // shape with one outline, which is the whole point.
        let morphing = *morph_progress > 0.0 && !morph_to.is_empty();
        let wants_field = styled || large || morphing;
        let origin = (
            bounds.x as f32 * scale,
            (bounds.y + vertical_offset) as f32 * scale,
        );
        let own = text_system.rasterize(*node, origin, scale, wants_field);
        // The target is shaped against the same box, so paired glyphs travel
        // between the positions the two texts actually put them in rather than
        // morphing in place and landing wrong.
        let target = if morphing {
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
                },
            );
            text_system.rasterize_target(*node, origin, scale, true)
        } else {
            Vec::new()
        };

        let mut push = |glyph: RasterGlyph, morph: Option<RasterGlyph>, progress: f32| {
            if glyph.width > 0 && glyph.height > 0 {
                glyphs.push(PreparedGlyph {
                    glyph,
                    morph,
                    morph_progress: progress,
                    color: *color,
                    color_overlay: *color_overlay,
                    transform: *transform,
                    command_index,
                    field: glyph_field_uniform(*field_style, *size),
                    outline_color: color_array(field_style.outline_color),
                });
            }
        };

        let progress = if morphing { *morph_progress } else { 0.0 };
        let mut target = target.into_iter();
        for glyph in own {
            // An unpaired letter keeps its progress and has no partner, so the
            // shader interpolates it towards empty space and it dissolves.
            push(glyph, target.next(), progress);
        }
        // Whatever the target has left over is arriving rather than leaving, so
        // it runs the same interpolation backwards: fully dissolved at zero and
        // whole at one.
        for glyph in target {
            push(glyph, None, 1.0 - progress);
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
        let box_of = |glyph: &RasterGlyph| Geometry {
            x: f64::from(glyph.x) / f64::from(scale),
            y: f64::from(glyph.y) / f64::from(scale),
            // The quad, not the bitmap: a distance field is measured once
            // and drawn at any size, so these are the only two of the four
            // that follow the font size rather than the atlas.
            width: f64::from(glyph.draw_width) / f64::from(scale),
            height: f64::from(glyph.draw_height) / f64::from(scale),
        };
        let mut quad = box_of(&glyph);
        // Paired glyphs are read through one quad, so it has to be the quad
        // both of them live in: the two letters differ in width and in where
        // they sit on the baseline, and sampling either field outside its own
        // box would clip the letter it is turning into.
        if let Some(partner) = &prepared.morph {
            let travel = f64::from(prepared.morph_progress);
            let target = box_of(partner);
            quad.x += (target.x - quad.x) * travel;
            quad.y += (target.y - quad.y) * travel;
            quad.width += (target.width - quad.width) * travel;
            quad.height += (target.height - quad.height) * travel;
        }
        let (origin, axes) = transformed_quad(
            prepared.transform,
            quad,
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
            ..GlyphInstance::default()
        });
    }
    Ok(Some(GlyphBatch {
        instances,
        command_spans,
    }))
}
