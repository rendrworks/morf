// Turning a node's shaped buffer into positioned glyphs.
//
// Shaping says which glyphs and where; this walks that layout and asks for each
// one as either a distance field or a direct rasterization. The pairing for a
// morph lives here too, because a pair is two runs read side by side.

use cosmic_text::PhysicalGlyph;
use morf_scene::NodeHandle;

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

    /// A node's glyphs paired with the glyphs of the text it is morphing into.
    ///
    /// Each pair is measured over one shared box, so the two fields are in the
    /// same units and can be interpolated; a glyph with no partner comes back
    /// alone and is left to dissolve. The pairing is positional, which is the
    /// only correspondence two arbitrary letters have.
    pub fn rasterize_pairs(
        &mut self,
        node: NodeHandle,
        origin: (f32, f32),
        scale: f32,
    ) -> Vec<GlyphPair> {
        let own = self.physical_glyphs(BufferKey::own(node), origin, scale);
        let target = self.physical_glyphs(BufferKey::target(node), origin, scale);
        let mut target = target.into_iter();
        own.into_iter()
            .map(|glyph| match target.next() {
                Some(partner) => match self.field_pair(&glyph, &partner) {
                    Some((first, second, apart)) => (Some(first), Some(second), apart),
                    None => (self.raster_glyph(&glyph, true), None, 0.0),
                },
                None => (self.raster_glyph(&glyph, true), None, 0.0),
            })
            .filter_map(|(glyph, partner, apart)| Some((glyph?, partner, apart)))
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
