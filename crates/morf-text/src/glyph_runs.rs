// Turning a node's shaped buffer into positioned glyphs.
//
// Shaping says which glyphs and where; this walks that layout and asks for each
// one as either a distance field or a direct rasterization. The pairing for a
// morph lives here too, because a pair is two runs read side by side.

use cosmic_text::PhysicalGlyph;
use morf_scene::NodeHandle;

use crate::glyph_morph::{Contour, contour_points, contours, pair_up, walk};
use crate::raster_glyph::field_raster;
use crate::{BufferKey, FastMap, GlyphPair, RasterGlyph, TextSystem};

impl TextSystem {
    /// Rasterizes one cached text node at a physical origin and scale.
    /// The glyphs of a laid-out node, positioned.
    ///
    /// `field` asks for distance-field glyphs rather than direct
    /// rasterizations. See `raster_glyph` for when that
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
        family: &str,
        family_to: &str,
    ) -> Vec<(f32, f32)> {
        let Some(from) = self.outline_points(glyph, family) else {
            return Vec::new();
        };
        // The two ends need not be the same face. Correspondence is geometry —
        // contours paired by position, resampled, rotated onto each other — so
        // one face's letter walks onto another's the same way it walks onto its
        // own. A face change is a morph, not a swap.
        let target = morph_to
            .filter(|other| *other != glyph || family_to != family)
            .filter(|_| travel > 0.0)
            .and_then(|other| self.outline_points(other, family_to));
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
    fn outline_key(&mut self, glyph: char, family: &str) -> Option<cosmic_text::CacheKey> {
        if let Some(known) = self.outline_keys.get(family).and_then(|by| by.get(&glyph)) {
            return *known;
        }
        let key = self.shape_one_in(glyph, family);
        if let Some(by_glyph) = self.outline_keys.get_mut(family) {
            by_glyph.insert(glyph, key);
        } else {
            let mut by_glyph = FastMap::default();
            by_glyph.insert(glyph, key);
            self.outline_keys.insert(family.into(), by_glyph);
        }
        key
    }

    fn shape_one_in(&mut self, glyph: char, family: &str) -> Option<cosmic_text::CacheKey> {
        let size = crate::glyph_fields::FIELD_REFERENCE_PX;
        let mut buffer =
            cosmic_text::Buffer::new(&mut self.fonts, cosmic_text::Metrics::relative(size, 1.2));
        let family = crate::resolve_family(&self.fonts, family);
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
        self.probe_outline_key_in(glyph, reference, "sans-serif")
    }

    #[cfg(test)]
    pub(crate) fn probe_outline_key_in(
        &mut self,
        glyph: char,
        reference: f32,
        family: &str,
    ) -> Option<cosmic_text::CacheKey> {
        let mut key = self.shape_one_in(glyph, family)?;
        key.font_size_bits = reference.to_bits();
        Some(key)
    }

    /// Debug: compare a corner's half-planes against the true distance around it.
    #[cfg(test)]
    pub(crate) fn probe_corner_sanity(&mut self, glyph: char, reference: f32) {
        use crate::glyph_fields::flatten;
        let Some(key) = self.probe_outline_key(glyph, reference) else {
            return;
        };
        let Some(commands) = self.probe_outline_commands(key) else {
            return;
        };
        let segments = flatten(&commands);
        let corners = crate::glyph_corners::corners(&commands);
        println!("{} corners found", corners.len());
        for corner in corners.iter().take(3) {
            println!(
                "corner at ({:.1},{:.1}) convex={} n0=({:.2},{:.2}) n1=({:.2},{:.2})",
                corner.at.0,
                corner.at.1,
                corner.convex,
                corner.normals[0].0,
                corner.normals[0].1,
                corner.normals[1].0,
                corner.normals[1].1
            );
            for (dx, dy) in [
                (1.0, 0.0),
                (-1.0, 0.0),
                (0.0, 1.0),
                (0.0, -1.0),
                (1.0, 1.0),
                (-1.0, -1.0),
            ] {
                let (x, y) = (corner.at.0 + dx, corner.at.1 + dy);
                let mut nearest = f32::MAX;
                let mut winding = 0;
                for piece in &segments {
                    nearest = nearest.min(piece.distance_squared(x, y));
                    winding += piece.winding(x, y);
                }
                let truth = if winding != 0 {
                    -nearest.sqrt()
                } else {
                    nearest.sqrt()
                };
                println!(
                    "    offset ({dx:+.0},{dy:+.0})  true {truth:+.3}  halfplanes {:+.3}",
                    corner.distance(x, y)
                );
            }
        }
    }

    /// Gate B: how much of the stored field's error a corner cell removes.
    ///
    /// Reads the field back exactly as the shader does — bilinear, at texel
    /// centres — and then, at texels near a corner, replaces the reading with
    /// the two half-planes that meet there, faded back to the field over an
    /// annulus. Reports the worst disagreement with the true distance inside
    /// the spread, before and after.
    #[cfg(test)]
    pub(crate) fn probe_corner_cells(
        &mut self,
        glyph: char,
        reference: f32,
        family: &str,
    ) -> Option<(f32, f32)> {
        use crate::glyph_fields::{field_spread_for, flatten, glyph_field};

        let key = self.probe_outline_key_in(glyph, reference, family)?;
        let commands = self.probe_outline_commands(key)?;
        let spread = field_spread_for(reference);
        let field = glyph_field(&commands, spread)?;
        let segments = flatten(&commands);
        let corners = crate::glyph_corners::corners(&commands);

        // How near a corner a texel has to be for its half-planes to speak, and
        // how far out their word fades back to the stored field.
        const REACH: f32 = 1.0;

        let width = field.width as usize;
        let height = field.height as usize;
        let read = |x: f32, y: f32| -> f32 {
            let fx = (x - field.left - 0.5).clamp(0.0, width as f32 - 1.001);
            let fy = (field.top - y - 0.5).clamp(0.0, height as f32 - 1.001);
            let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let at = |cx: usize, cy: usize| {
                f32::from(field.data[cy.min(height - 1) * width + cx.min(width - 1)]) / 255.0
            };
            let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
            let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
            ((top * (1.0 - ty) + bottom * ty) * 2.0 - 1.0) * spread
        };

        // Sampled between texel centres, not on them. On a centre the stored
        // value is returned verbatim and bilinear has done nothing yet — which
        // measures the storage and not the reading, and reports a tenth of the
        // real error.
        const STEP: f32 = 1.0 / 3.0;
        let mut worst_plain = 0.0_f32;
        let mut worst_corrected = 0.0_f32;
        let across = (width as f32 / STEP) as usize;
        let down = (height as f32 / STEP) as usize;
        for row in 0..down {
            for column in 0..across {
                let x = field.left + column as f32 * STEP;
                let y = field.top - row as f32 * STEP;

                let mut nearest = f32::MAX;
                let mut winding = 0;
                for piece in &segments {
                    nearest = nearest.min(piece.distance_squared(x, y));
                    winding += piece.winding(x, y);
                }
                let truth = if winding != 0 {
                    -nearest.sqrt()
                } else {
                    nearest.sqrt()
                };
                if truth.abs() >= spread {
                    continue;
                }

                let stored = read(x, y);
                worst_plain = worst_plain.max((stored - truth).abs());

                // The nearest corner, and only where it is the nearest thing
                // there is.
                //
                // A wedge describes the shape only inside the region the corner
                // owns — where one of its two edges really is the closest part
                // of the outline. Across a thin stroke the closest part is the
                // far side, and the wedge there says something confident and
                // wrong. Two cheap tests stand in for the region: the point is
                // near the corner, and it is near the *outline* — which the
                // stored value already says, and which is free.
                let mut closest = f32::MAX;
                let mut corrected = stored;
                for corner in &corners {
                    let apart = ((x - corner.at.0).powi(2) + (y - corner.at.1).powi(2)).sqrt();
                    if apart >= closest || apart >= REACH * 2.0 {
                        continue;
                    }
                    closest = apart;
                    if stored.abs() > REACH {
                        corrected = stored;
                        continue;
                    }
                    let wedge = corner.distance(x, y);
                    // The corner cannot be the nearest feature if it disagrees
                    // with the field about which side of the outline this is.
                    if (wedge < 0.0) != (stored < 0.0) && stored.abs() > 0.5 {
                        corrected = stored;
                        continue;
                    }
                    let blend = ((apart - REACH) / REACH).clamp(0.0, 1.0);
                    corrected = wedge * (1.0 - blend) + stored * blend;
                }
                worst_corrected = worst_corrected.max((corrected - truth).abs());
            }
        }
        Some((worst_plain, worst_corrected))
    }

    /// Diagnostic: how far the resampled contours stray from the true outline.
    ///
    /// One-way Hausdorff, outline to resample, in glyph units. This is the
    /// number that says whether a corner survived being spaced evenly along a
    /// loop — a curve resamples to almost nothing, a corner to whatever the
    /// nearest sample happened to be.
    #[cfg(test)]
    pub(crate) fn probe_resample_error(
        &mut self,
        glyph: char,
        reference: f32,
        family: &str,
    ) -> Option<f32> {
        let key = self.probe_outline_key_in(glyph, reference, family)?;
        let commands = self.probe_outline_commands(key)?;
        let contours = crate::glyph_morph::contours(&commands);
        if contours.is_empty() {
            return None;
        }
        let walked: Vec<(f32, f32)> = contours
            .iter()
            .flat_map(crate::glyph_morph::contour_of)
            .copied()
            .collect();
        let mut worst = 0.0_f32;
        for piece in crate::glyph_fields::flatten(&commands) {
            for point in [(piece.x0, piece.y0), (piece.x1, piece.y1)] {
                let mut nearest = f32::MAX;
                for index in 0..walked.len() {
                    let a = walked[index];
                    let b = walked[(index + 1) % walked.len()];
                    let edge = (b.0 - a.0, b.1 - a.1);
                    let length = edge.0 * edge.0 + edge.1 * edge.1;
                    let along = if length <= f32::EPSILON {
                        0.0
                    } else {
                        (((point.0 - a.0) * edge.0 + (point.1 - a.1) * edge.1) / length)
                            .clamp(0.0, 1.0)
                    };
                    let (dx, dy) = (
                        a.0 + edge.0 * along - point.0,
                        a.1 + edge.1 * along - point.1,
                    );
                    nearest = nearest.min(dx * dx + dy * dy);
                }
                worst = worst.max(nearest.sqrt());
            }
        }
        Some(worst)
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

    fn outline_points(&mut self, glyph: char, family: &str) -> Option<Vec<Contour>> {
        let key = self.outline_key(glyph, family)?;
        let commands = self.glyphs.get_outline_commands(&mut self.fonts, key)?;
        let found = contours(commands);
        (!found.is_empty()).then_some(found)
    }
}
