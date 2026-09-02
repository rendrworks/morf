//! Outlines kept, so a document is read once rather than once a frame.
//!
//! Parsing an SVG is not free, and neither is resampling its loops — but both
//! answer the same for as long as the file does not change. What a field asks
//! for sixty times a second is the walk between two outlines at some fraction,
//! and that is the only part that has to happen per frame.

use std::collections::HashMap;

use morf_outline::{Contour, Paired, contour_points, contours, pair_up, walk};

use crate::{Outline, SvgError, outline_of};

/// Documents already read, by the path they were read from.
#[derive(Default)]
pub struct SvgOutlines {
    loops: HashMap<Box<str>, Option<Vec<Contour>>>,
    /// Correspondences already worked out, by the pair that made them. Pairing
    /// two outlines is the expensive half — every loop of one is matched to a
    /// loop of the other and then rotated onto it — and it does not change as
    /// the morph runs.
    paired: HashMap<(Box<str>, Box<str>), Vec<Paired>>,
}

impl SvgOutlines {
    pub fn new() -> Self {
        Self::default()
    }

    /// The points of one drawing, or of the shape partway between two.
    ///
    /// The same call a letter answers, deliberately: a field layer holds an
    /// outline and does not care which kind of file it was written in.
    pub fn outline(
        &mut self,
        source: &str,
        morph_to: Option<&str>,
        travel: f32,
    ) -> Vec<(f32, f32)> {
        if self.read(source).is_none() {
            return Vec::new();
        }
        let target = morph_to
            .filter(|other| *other != source)
            .filter(|_| travel > 0.0)
            .filter(|other| self.read(other).is_some());
        match target {
            Some(other) => {
                let key = (Box::from(source), Box::from(other));
                if !self.paired.contains_key(&key) {
                    let from = self.loops[source].clone().unwrap_or_default();
                    let to = self.loops[other].clone().unwrap_or_default();
                    self.paired.insert(key.clone(), pair_up(from, to));
                }
                walk(&self.paired[&key], travel.clamp(0.0, 1.0))
            }
            None => contour_points(self.loops[source].as_deref().unwrap_or_default()),
        }
    }

    /// Whether a document has an outline in it, reading it if this is the first
    /// time it has been named. A file that cannot be read is remembered as such
    /// so a broken path is not re-opened every frame.
    fn read(&mut self, source: &str) -> Option<&Vec<Contour>> {
        if !self.loops.contains_key(source) {
            let read = outline_of(source)
                .ok()
                .map(|outline: Outline| contours(&outline.steps))
                .filter(|loops| !loops.is_empty());
            self.loops.insert(Box::from(source), read);
        }
        self.loops.get(source).and_then(Option::as_ref)
    }

    /// One drawing's closed loops, for a caller pairing them with something
    /// that is not a drawing — a letter, most usefully.
    pub fn contours_of(&mut self, source: &str) -> Vec<Contour> {
        self.read(source).cloned().unwrap_or_default()
    }

    /// The walk between two sets of loops that came from different places.
    ///
    /// Kept here only because something has to own the pairing; nothing in it
    /// is about drawings. Two outlines correspond or they do not, and a letter
    /// and an icon correspond exactly as well as two letters do.
    pub fn walk_between(from: Vec<Contour>, to: Vec<Contour>, travel: f32) -> Vec<(f32, f32)> {
        walk(&pair_up(from, to), travel.clamp(0.0, 1.0))
    }

    /// Forgets everything, for a configuration reloaded from disk.
    pub fn clear(&mut self) {
        self.loops.clear();
        self.paired.clear();
    }

    /// Reads a document without keeping it, for a caller that wants the error.
    pub fn probe(source: &str) -> Result<Outline, SvgError> {
        outline_of(source)
    }
}
