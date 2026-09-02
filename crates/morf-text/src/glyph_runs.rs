// Turning a node's shaped buffer into positioned glyphs.
//
// Shaping says which glyphs and where; this walks that layout and asks for each
// one as either a distance field or a direct rasterization. The pairing for a
// morph lives here too, because a pair is two runs read side by side.

use cosmic_text::PhysicalGlyph;
use morf_scene::NodeHandle;

use crate::glyph_morph::{Contour, contour_points, contours, pair_up, walk};
use crate::raster_glyph::field_raster;
use crate::{BufferKey, GlyphPair, RasterGlyph, TextSystem};

impl TextSystem {
    /// Rasterizes one cached text node at a physical origin and scale.
    /// The glyphs of a laid-out node, positioned.
    ///
    /// `field` asks for distance-field glyphs rather than direct
    /// rasterizations. See [`raster_glyph`](Self::raster_glyph) for when that
    /// is the right thing to want; for ordinary text at its own size it is not.
    pub fn rasterize(
        &mut self,
        node: NodeHandle,
        origin: (f32, f32),
        scale: f32,
        field: bool,
    ) -> Vec<RasterGlyph> {
        self.rasterize_run(BufferKey::own(node), origin, scale, field)
    }

    /// The glyphs of the text a node is morphing *towards*, positioned the same
    /// way. Empty when the node is not morphing, because nothing shaped it.
    pub fn rasterize_target(
        &mut self,
        node: NodeHandle,
        origin: (f32, f32),
        scale: f32,
        field: bool,
    ) -> Vec<RasterGlyph> {
        self.rasterize_run(BufferKey::target(node), origin, scale, field)
    }

    /// A node's glyphs, each with the shape it is part way towards.
    ///
    /// What comes back is not the two letters: it is the two *frames* of the
    /// morph either side of `travel`, and how far between them the glyph is.
    /// The correspondence between the letters was solved in the outline when
    /// the frames were measured, so all that is left here is to pick a pair
    /// that already differ by almost nothing.
    pub fn rasterize_pairs(
        &mut self,
        node: NodeHandle,
        origin: (f32, f32),
        scale: f32,
        travel: f32,
    ) -> Vec<GlyphPair> {
        let own = self.physical_glyphs(BufferKey::own(node), origin, scale);
        let target = self.physical_glyphs(BufferKey::target(node), origin, scale);
        let mut target = target.into_iter();
        own.into_iter()
            .map(|glyph| {
                let Some(partner) = target.next() else {
                    return (self.raster_glyph(&glyph, true), None, 0.0);
                };
                let Some(from_key) = self.morph_frames(&glyph, &partner) else {
                    return (self.raster_glyph(&glyph, true), None, 0.0);
                };
                let to_key = Self::pair_target_key(&partner);
                match self.morph_step(from_key, to_key, travel) {
                    Some((first, first_key, second, second_key, local)) => (
                        Some(field_raster(&glyph, first_key, &first)),
                        Some(field_raster(&glyph, second_key, &second)),
                        local,
                    ),
                    None => (self.raster_glyph(&glyph, true), None, 0.0),
                }
            })
            .filter_map(|(glyph, partner, local)| Some((glyph?, partner, local)))
            .filter(|(glyph, _, _)| glyph.width > 0 && glyph.height > 0)
            .collect()
    }

    fn physical_glyphs(
        &mut self,
        key: BufferKey,
        origin: (f32, f32),
        scale: f32,
    ) -> Vec<PhysicalGlyph> {
        let Some(buffer) = self.buffers.get(&key) else {
            return Vec::new();
        };
        buffer
            .buffer
            .layout_runs()
            .flat_map(|run| {
                run.glyphs.iter().map(move |glyph| {
                    glyph.physical((origin.0, origin.1 + run.line_y * scale), scale)
                })
            })
            .collect()
    }

    fn rasterize_run(
        &mut self,
        key: BufferKey,
        origin: (f32, f32),
        scale: f32,
        field: bool,
    ) -> Vec<RasterGlyph> {
        let Some(buffer) = self.buffers.get(&key) else {
            return Vec::new();
        };
        let physical: Vec<_> = buffer
            .buffer
            .layout_runs()
            .flat_map(|run| {
                run.glyphs.iter().map(move |glyph| {
                    glyph.physical((origin.0, origin.1 + run.line_y * scale), scale)
                })
            })
            .collect();
        physical
            .into_iter()
            .filter_map(|glyph| self.raster_glyph(&glyph, field))
            .collect()
    }
}

impl TextSystem {
    /// One character's outline as points, optionally part way to another.
    ///
    /// This is how a letter becomes a shape a distance field can compose with.
    /// It is not a picture of a letter sampled from an atlas — it is the
    /// outline itself, so it unions, subtracts and morphs with a circle by the
    /// same arithmetic a circle does, at whatever size it is drawn.
    ///
    /// A morphing pair is walked here rather than in the shader: the
    /// correspondence between the two letters is a property of the outlines and
    /// costs a few hundred multiplications to apply, so what reaches the GPU is
    /// one outline and a morphing letter costs a still one's price.
    pub fn glyph_outline(
        &mut self,
        glyph: char,
        morph_to: Option<char>,
        travel: f32,
    ) -> Vec<(f32, f32)> {
        let Some(from) = self.outline_points(glyph) else {
            return Vec::new();
        };
        let target = morph_to
            .filter(|other| *other != glyph && travel > 0.0)
            .and_then(|other| self.outline_points(other));
        match target {
            Some(to) => walk(&pair_up(from, to), travel.clamp(0.0, 1.0)),
            None => contour_points(&from),
        }
    }

    /// The cache key one character's outline is measured under.
    ///
    /// A character has to be shaped before a font can be asked for its outline,
    /// and shaping wants a buffer. This keeps one for the purpose rather than
    /// borrowing a node's, since a letter used as a shape belongs to no text.
    fn outline_key(&mut self, glyph: char) -> Option<cosmic_text::CacheKey> {
        if let Some(known) = self.outline_keys.get(&glyph) {
            return *known;
        }
        let key = self.shape_one(glyph);
        self.outline_keys.insert(glyph, key);
        key
    }

    fn shape_one(&mut self, glyph: char) -> Option<cosmic_text::CacheKey> {
        let size = crate::glyph_fields::FIELD_REFERENCE_PX;
        let mut buffer =
            cosmic_text::Buffer::new(&mut self.fonts, cosmic_text::Metrics::relative(size, 1.2));
        let family = crate::resolve_family(&self.fonts, "sans-serif");
        buffer.set_text(
            glyph.encode_utf8(&mut [0u8; 4]),
            &cosmic_text::Attrs::new().family(family.family()),
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        let mut key = buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter())
            .map(|glyph| glyph.physical((0.0, 0.0), 1.0).cache_key)
            .next()?;
        key.font_size_bits = size.to_bits();
        key.x_bin = cosmic_text::SubpixelBin::Zero;
        key.y_bin = cosmic_text::SubpixelBin::Zero;
        Some(key)
    }

    /// Diagnostic access to the shaped key and its outline, at a chosen size.
    ///
    /// For the probe that writes a glyph twice — as the outline traces it and
    /// as the field reconstructs it — which is how the two are told apart when
    /// one of them looks wrong.
    #[cfg(test)]
    pub(crate) fn probe_outline_key(
        &mut self,
        glyph: char,
        reference: f32,
    ) -> Option<cosmic_text::CacheKey> {
        let mut key = self.outline_key(glyph)?;
        key.font_size_bits = reference.to_bits();
        Some(key)
    }

    #[cfg(test)]
    pub(crate) fn probe_outline_commands(
        &mut self,
        key: cosmic_text::CacheKey,
    ) -> Option<Vec<cosmic_text::Command>> {
        self.glyphs
            .get_outline_commands(&mut self.fonts, key)
            .map(<[cosmic_text::Command]>::to_vec)
    }

    fn outline_points(&mut self, glyph: char) -> Option<Vec<Contour>> {
        let key = self.outline_key(glyph)?;
        let commands = self.glyphs.get_outline_commands(&mut self.fonts, key)?;
        let found = contours(commands);
        (!found.is_empty()).then_some(found)
    }
}
