// Turning one shaped glyph into something the atlas can hold.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use cosmic_text::{PhysicalGlyph, SubpixelBin, SwashContent};

use crate::MORPH_FRAMES;
use crate::glyph_fields::{
    FieldImage, field_from_segments, field_reference_for, field_spread_for, glyph_field,
    segment_box,
};
use crate::glyph_morph::{between, contours, pair_up};
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
    /// One glyph, as a distance field or as a direct rasterization.
    ///
    /// `field` says whether a field is *wanted*. It is not free: the field is
    /// measured once at [`FIELD_REFERENCE_PX`] and scaled to whatever size is
    /// asked for, so an eleven-pixel label drawn from a sixty-four-pixel field
    /// arrives with no hinting and a soft edge — which is worse than the
    /// direct rasterization it replaced, at the size most text is actually
    /// drawn. The field earns its keep above the reference size, where a
    /// direct raster would need its own cache entry per size, and whenever a
    /// style asks for something only a field can do.
    pub(crate) fn raster_glyph(
        &mut self,
        glyph: &PhysicalGlyph,
        field: bool,
    ) -> Option<RasterGlyph> {
        if !field {
            return self.mask_glyph(glyph);
        }
        let (reference, key) = Self::field_key(glyph);
        let spread = field_spread_for(f32::from_bits(reference.font_size_bits));
        if !self.fields.contains_key(&key) {
            // Straight from the font's own curves. A colour glyph has no
            // outline to ask for and comes back `None`, which falls through to
            // the rasterizer below — an emoji is a picture, and there is no
            // edge in a picture to measure a distance from.
            let measured = self
                .glyphs
                .get_outline_commands(&mut self.fonts, reference)
                .and_then(|commands| glyph_field(commands, spread))
                .map(Rc::new);
            self.fields.insert(key, measured);
        }

        if let Some(field) = self.fields.get(&key).and_then(Option::as_ref) {
            return Some(field_raster(glyph, key, field));
        }

        self.mask_glyph(glyph)
    }

    /// The reference cache key a glyph's field is measured under.
    ///
    /// Size and subpixel offset are stripped: a field records the shape, and
    /// the shape does not change with either.
    fn field_key(glyph: &PhysicalGlyph) -> (cosmic_text::CacheKey, u64) {
        let mut reference = glyph.cache_key;
        let drawn = f32::from_bits(glyph.cache_key.font_size_bits);
        reference.font_size_bits = field_reference_for(drawn).to_bits();
        reference.x_bin = SubpixelBin::Zero;
        reference.y_bin = SubpixelBin::Zero;
        let mut hasher = DefaultHasher::new();
        reference.hash(&mut hasher);
        (reference, hasher.finish())
    }

    /// Two glyphs measured over one shared box, for interpolating between them.
    ///
    /// Separately measured fields cannot be interpolated: each is in units of
    /// its own box, and the shader reads both through one set of texture
    /// coordinates. Measuring both over the union of the two boxes is what
    /// makes the two fields comparable — and it is only affordable because the
    /// field is measured from the outline, where the box is a choice, rather
    /// than inherited from whatever rectangle a rasterizer happened to return.
    /// The reference key a partner glyph's frames are stored under.
    pub(crate) fn pair_target_key(glyph: &PhysicalGlyph) -> u64 {
        Self::field_key(glyph).1
    }

    /// Two glyphs and the shapes between them, as a strip of measured frames.
    ///
    /// The morph is solved in the outline — contours matched, resampled and
    /// rotated onto each other, then walked point by point — and each step is
    /// measured into a field of its own. What the renderer interpolates is two
    /// *neighbouring* steps, which differ by a fraction of the journey, so the
    /// interpolation has almost nothing to do and none of the swelling that
    /// averaging the two end letters produces.
    ///
    /// Every frame shares one box, so the strip is read through one quad.
    pub(crate) fn morph_frames(&mut self, from: &PhysicalGlyph, to: &PhysicalGlyph) -> Option<u64> {
        let (from_reference, from_key) = Self::field_key(from);
        let (to_reference, to_key) = Self::field_key(to);
        let pair = (from_key, to_key);
        if !self.field_pairs.contains_key(&pair) {
            let measured = self.measure_frames(from_reference, to_reference);
            self.field_pairs.insert(pair, measured);
        }
        self.field_pairs.get(&pair)?.as_ref().map(|_| from_key)
    }

    /// The atlas key one frame of a morph is held under.
    ///
    /// Every frame is its own picture and needs its own entry. Keying them all
    /// by the two letters put thirteen different shapes under two names, and
    /// the atlas — quite correctly — kept the first of each and handed it back
    /// for the rest, so the morph never moved.
    fn frame_key(from_key: u64, to_key: u64, index: usize) -> u64 {
        from_key
            .wrapping_mul(0x9e3779b97f4a7c15)
            .wrapping_add(to_key.wrapping_mul(0xc2b2ae3d27d4eb4f))
            .wrapping_add(index as u64)
    }

    /// The two frames either side of `travel`, and how far between them it is.
    pub(crate) fn morph_step(
        &self,
        from_key: u64,
        to_key: u64,
        travel: f32,
    ) -> Option<crate::MorphStep> {
        let frames = self.field_pairs.get(&(from_key, to_key))?.as_ref()?;
        let last = frames.len() - 1;
        let along = (travel.clamp(0.0, 1.0) * last as f32).clamp(0.0, last as f32);
        let index = (along.floor() as usize).min(last.saturating_sub(1));
        let next = (index + 1).min(last);
        Some((
            Rc::clone(&frames[index]),
            Self::frame_key(from_key, to_key, index),
            Rc::clone(&frames[next]),
            Self::frame_key(from_key, to_key, next),
            along - index as f32,
        ))
    }

    fn measure_frames(
        &mut self,
        from: cosmic_text::CacheKey,
        to: cosmic_text::CacheKey,
    ) -> Option<Vec<Rc<FieldImage>>> {
        let from_commands = self
            .glyphs
            .get_outline_commands(&mut self.fonts, from)?
            .to_vec();
        let to_commands = self
            .glyphs
            .get_outline_commands(&mut self.fonts, to)?
            .to_vec();
        let spread = field_spread_for(f32::from_bits(from.font_size_bits));
        let paired = pair_up(contours(&from_commands), contours(&to_commands));
        if paired.is_empty() {
            return None;
        }

        // One box for the whole strip, taken from the widest the shape ever
        // gets on the way across — which is not always either end of it.
        let mut area: Option<crate::glyph_fields::FieldBox> = None;
        let mut steps = Vec::with_capacity(MORPH_FRAMES);
        for frame in 0..MORPH_FRAMES {
            let travel = frame as f32 / (MORPH_FRAMES - 1) as f32;
            let segments = between(&paired, travel);
            let box_here = segment_box(&segments, spread)?;
            area = Some(match area {
                Some(known) => known.union(box_here),
                None => box_here,
            });
            steps.push(segments);
        }
        let area = area?;

        let mut frames = Vec::with_capacity(steps.len());
        for segments in steps {
            frames.push(Rc::new(field_from_segments(&segments, area, spread)?));
        }
        Some(frames)
    }

    /// A glyph rasterized at the size it is drawn.
    ///
    /// Hinted, crisp, and one cache entry per size — which is the right trade
    /// for body text, and the wrong one for a glyph being animated through a
    /// range of sizes.
    fn mask_glyph(&mut self, glyph: &PhysicalGlyph) -> Option<RasterGlyph> {
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

/// One measured field, placed against a pen position and a drawn size.
pub(crate) fn field_raster(glyph: &PhysicalGlyph, key: u64, field: &Rc<FieldImage>) -> RasterGlyph {
    // How much bigger than the reference this glyph is being drawn.
    let drawn = f32::from_bits(glyph.cache_key.font_size_bits);
    let scale = drawn / field_reference_for(drawn);
    RasterGlyph {
        cache_key: key,
        x: glyph.x + (field.left as f32 * scale).round() as i32,
        y: glyph.y - (field.top as f32 * scale).round() as i32,
        width: field.width,
        height: field.height,
        draw_width: (field.width as f32 * scale).round().max(1.0) as u32,
        draw_height: (field.height as f32 * scale).round().max(1.0) as u32,
        content: RasterContent::Field,
        data: Rc::clone(&field.data),
    }
}
