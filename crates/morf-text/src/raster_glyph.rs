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
use crate::{MeasuredPair, RasterContent, RasterGlyph, TextSystem};

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

    /// Measures, across every core, the frames the morphs in `wanted` need
    /// at `travel` and have not got yet. A field is pure geometry -- the
    /// paired outlines walked to one step and rasterised -- so a word of new
    /// pairs is measured in the time of one glyph rather than one after
    /// another on the frame's thread.
    pub(crate) fn prepare_morph_frames(&mut self, wanted: &[(u64, u64)], travel: f32) {
        // One frame to measure: which pair, which step, and what to measure it from.
        type Job = (
            (u64, u64),
            usize,
            Vec<morf_outline::Paired>,
            crate::glyph_fields::FieldBox,
            f32,
        );
        let mut jobs: Vec<Job> = Vec::new();
        for key in wanted {
            let Some(Some(pair)) = self.field_pairs.get(key) else {
                continue;
            };
            let last = pair.frames.len() - 1;
            let along = (travel.clamp(0.0, 1.0) * last as f32).clamp(0.0, last as f32);
            let index = (along.floor() as usize).min(last.saturating_sub(1));
            for frame in [index, (index + 1).min(last)] {
                if pair.frames[frame].is_none()
                    && !jobs.iter().any(|job| job.0 == *key && job.1 == frame)
                {
                    jobs.push((*key, frame, pair.paired.clone(), pair.area, pair.spread));
                }
            }
        }
        if jobs.is_empty() {
            return;
        }
        let last = (MORPH_FRAMES - 1) as f32;
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .clamp(1, jobs.len());
        let per_thread = jobs.len().div_ceil(threads);
        // A field's texels travel back as a plain vector; the shared handle
        // around them is made on this thread, where it is used.
        type Measured = ((u64, u64), usize, Option<(f32, f32, u32, u32, Vec<u8>)>);
        let measured: Vec<Measured> = std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .chunks(per_thread)
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|(key, frame, paired, area, spread)| {
                                let segments = between(paired, *frame as f32 / last);
                                let parts =
                                    field_from_segments(&segments, *area, *spread).map(|image| {
                                        let texels = Rc::try_unwrap(image.data)
                                            .unwrap_or_else(|rc| (*rc).clone());
                                        (image.left, image.top, image.width, image.height, texels)
                                    });
                                (*key, *frame, parts)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap_or_default())
                .collect()
        });
        for (key, frame, parts) in measured {
            if let (Some(Some(pair)), Some((left, top, width, height, texels))) =
                (self.field_pairs.get_mut(&key), parts)
            {
                pair.frames[frame] = Some(Rc::new(FieldImage {
                    left,
                    top,
                    width,
                    height,
                    data: Rc::new(texels),
                }));
            }
        }
    }

    /// The two frames either side of `travel`, and how far between them it is.
    /// A frame not yet measured is measured now, and kept.
    pub(crate) fn morph_step(
        &mut self,
        from_key: u64,
        to_key: u64,
        travel: f32,
    ) -> Option<crate::MorphStep> {
        let pair = self.field_pairs.get_mut(&(from_key, to_key))?.as_mut()?;
        let last = pair.frames.len() - 1;
        let along = (travel.clamp(0.0, 1.0) * last as f32).clamp(0.0, last as f32);
        let index = (along.floor() as usize).min(last.saturating_sub(1));
        let next = (index + 1).min(last);
        for frame in [index, next] {
            if pair.frames[frame].is_none() {
                let segments = between(&pair.paired, frame as f32 / last as f32);
                let image = field_from_segments(&segments, pair.area, pair.spread)?;
                pair.frames[frame] = Some(Rc::new(image));
            }
        }
        Some((
            Rc::clone(pair.frames[index].as_ref()?),
            Self::frame_key(from_key, to_key, index),
            Rc::clone(pair.frames[next].as_ref()?),
            Self::frame_key(from_key, to_key, next),
            along - index as f32,
        ))
    }

    /// Pairs two outlines and finds the box their morph needs: the widest
    /// the shape gets on the way across, which is not always either end.
    /// The frames are left for `morph_step` to measure as it reaches them.
    fn measure_frames(
        &mut self,
        from: cosmic_text::CacheKey,
        to: cosmic_text::CacheKey,
    ) -> Option<MeasuredPair> {
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
        let mut area: Option<crate::glyph_fields::FieldBox> = None;
        for frame in 0..MORPH_FRAMES {
            let travel = frame as f32 / (MORPH_FRAMES - 1) as f32;
            let box_here = segment_box(&between(&paired, travel), spread)?;
            area = Some(match area {
                Some(known) => known.union(box_here),
                None => box_here,
            });
        }
        Some(MeasuredPair {
            paired,
            area: area?,
            spread,
            frames: vec![None; MORPH_FRAMES],
        })
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
            x: (glyph.x + image.placement.left) as f32,
            y: (glyph.y - image.placement.top) as f32,
            width: image.placement.width,
            height: image.placement.height,
            draw_width: image.placement.width as f32,
            draw_height: image.placement.height as f32,
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
    // Where shaping actually put this glyph, fraction and all. A rasterizer
    // threw the fraction into a subpixel bin and rendered a variant for it; a
    // field is one shape drawn wherever it is told, so the fraction is simply
    // kept. Dropping it is what leaves letters standing unevenly apart.
    let pen_x = glyph.x as f32 + glyph.cache_key.x_bin.as_float();
    let pen_y = glyph.y as f32 + glyph.cache_key.y_bin.as_float();
    RasterGlyph {
        cache_key: key,
        x: pen_x + field.left * scale,
        y: pen_y - field.top * scale,
        width: field.width,
        height: field.height,
        draw_width: (field.width as f32 * scale).max(1.0),
        draw_height: (field.height as f32 * scale).max(1.0),
        content: RasterContent::Field,
        data: Rc::clone(&field.data),
    }
}
