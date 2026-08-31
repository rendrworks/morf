// Turning one shaped glyph into something the atlas can hold.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use cosmic_text::{PhysicalGlyph, SubpixelBin, SwashContent};

use crate::glyph_fields::{FIELD_REFERENCE_PX, FIELD_SPREAD_PX, FieldImage, glyph_field};
use crate::{RasterContent, RasterGlyph, TextSystem};

impl TextSystem {
    /// One positioned glyph, as a distance field wherever that is possible.
    ///
    /// The field is measured from the glyph rasterized at `FIELD_REFERENCE_PX`,
    /// never at the size being drawn, and the quad is that reference box scaled
    /// to the size being drawn. That is the whole point: the letter is stored
    /// once and read at any size, so animating a font size costs nothing but
    /// arithmetic where it used to refill the atlas every frame.
    ///
    /// Colour glyphs stay as they were. An emoji is a picture, not a shape, and
    /// there is no edge in it to measure a distance from.
    pub(crate) fn raster_glyph(&mut self, glyph: &PhysicalGlyph) -> Option<RasterGlyph> {
        let mut reference = glyph.cache_key;
        reference.font_size_bits = FIELD_REFERENCE_PX.to_bits();
        reference.x_bin = SubpixelBin::Zero;
        reference.y_bin = SubpixelBin::Zero;

        let mut hasher = DefaultHasher::new();
        reference.hash(&mut hasher);
        let key = hasher.finish();
        if !self.fields.contains_key(&key) {
            let measured = self
                .glyphs
                .get_image(&mut self.fonts, reference)
                .clone()
                .filter(|image| image.content != SwashContent::Color)
                .and_then(|image| {
                    let field =
                        glyph_field(&image.data, image.placement.width, image.placement.height)?;
                    Some(Rc::new(FieldImage {
                        // The placement moves with the padding the field added
                        // around the glyph, so the quad below covers the field
                        // rather than only the ink inside it.
                        left: image.placement.left - FIELD_SPREAD_PX as i32,
                        top: image.placement.top + FIELD_SPREAD_PX as i32,
                        ..field
                    }))
                });
            self.fields.insert(key, measured);
        }

        if let Some(field) = self.fields.get(&key).and_then(Option::as_ref) {
            // How much bigger than the reference this glyph is being drawn.
            let scale = f32::from_bits(glyph.cache_key.font_size_bits) / FIELD_REFERENCE_PX;
            return Some(RasterGlyph {
                cache_key: key,
                x: glyph.x + (field.left as f32 * scale).round() as i32,
                y: glyph.y - (field.top as f32 * scale).round() as i32,
                width: field.width,
                height: field.height,
                draw_width: (field.width as f32 * scale).round().max(1.0) as u32,
                draw_height: (field.height as f32 * scale).round().max(1.0) as u32,
                content: RasterContent::Field,
                data: Rc::clone(&field.data),
            });
        }

        // A colour glyph, or one with no ink to measure: rasterized at the size
        // it is drawn, the way everything used to be.
        let mut hasher = DefaultHasher::new();
        glyph.cache_key.hash(&mut hasher);
        let cache_key = hasher.finish();
        let image = self
            .glyphs
            .get_image(&mut self.fonts, glyph.cache_key)
            .clone()?;
        let content = match image.content {
            SwashContent::Color => RasterContent::Color,
            SwashContent::Mask | SwashContent::SubpixelMask => RasterContent::Mask,
        };
        Some(RasterGlyph {
            cache_key,
            x: glyph.x + image.placement.left,
            y: glyph.y - image.placement.top,
            width: image.placement.width,
            height: image.placement.height,
            draw_width: image.placement.width,
            draw_height: image.placement.height,
            content,
            data: Rc::new(image.data),
        })
    }
}
