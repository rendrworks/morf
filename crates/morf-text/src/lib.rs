//! Text shaping, measurement, and glyph rasterization for morf.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::rc::Rc;

use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use morf_layout::{TextAlignment, TextElide, TextOptions};
use morf_scene::{FastMap, NodeHandle};
use unicode_segmentation::UnicodeSegmentation;

use crate::glyph_fields::FieldImage;

struct CachedBuffer {
    buffer: Buffer,
    input: Option<TextInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ResolvedFamily {
    Name(String),
    Serif,
    SansSerif,
    Monospace,
    Cursive,
    Fantasy,
}

impl ResolvedFamily {
    fn family(&self) -> Family<'_> {
        match self {
            Self::Name(name) => Family::Name(name),
            Self::Serif => Family::Serif,
            Self::SansSerif => Family::SansSerif,
            Self::Monospace => Family::Monospace,
            Self::Cursive => Family::Cursive,
            Self::Fantasy => Family::Fantasy,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Name(name) => name,
            Self::Serif => "serif",
            Self::SansSerif => "sans-serif",
            Self::Monospace => "monospace",
            Self::Cursive => "cursive",
            Self::Fantasy => "fantasy",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TextInput {
    text: String,
    family: String,
    size: u64,
    width: Option<u64>,
    wrap: bool,
    alignment: TextAlignment,
    elide: TextElide,
    font_weight: u16,
    font_source: Option<String>,
}

/// Two glyphs measured over one box, and how much of them fails to overlap.
pub(crate) type MeasuredPair = Vec<Rc<FieldImage>>;

/// Two neighbouring frames of a morph, their atlas keys, and where between
/// them the glyph currently is.
pub(crate) type MorphStep = (Rc<FieldImage>, u64, Rc<FieldImage>, u64, f32);

/// How many shapes are measured along the way from one letter to the next.
///
/// The renderer interpolates the two either side of where it is, so this sets
/// how far apart those two are: enough of them that neighbours differ by a
/// twelfth of the journey, which is close enough that averaging their fields
/// has nothing left to get wrong.
pub(crate) const MORPH_FRAMES: usize = 13;

/// One glyph paired with the glyph it is turning into, if it has one, and how
/// far apart the two shapes are.
pub type GlyphPair = (RasterGlyph, Option<RasterGlyph>, f32);

/// Which of a node's two shaped runs a buffer holds.
///
/// A text node that is morphing has two: the text it is, and the text it is
/// turning into. Both have to stay shaped and measured, because the morph
/// interpolates between the two sets of glyphs rather than between two
/// pictures, so one key is not enough.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BufferKey {
    pub(crate) node: NodeHandle,
    pub(crate) morph: bool,
}

impl BufferKey {
    pub(crate) fn own(node: NodeHandle) -> Self {
        Self { node, morph: false }
    }

    pub(crate) fn target(node: NodeHandle) -> Self {
        Self { node, morph: true }
    }
}

/// Shared font database, per-node shaped buffers, and glyph image cache.
pub struct TextSystem {
    fonts: FontSystem,
    glyphs: SwashCache,
    buffers: FastMap<BufferKey, CachedBuffer>,
    /// Fields measured for a *pair* of glyphs over their shared box.
    ///
    /// Separate from `fields` because the box depends on both glyphs, so the
    /// same letter measured against two different partners is two entries.
    field_pairs: FastMap<(u64, u64), Option<MeasuredPair>>,
    /// Shaped keys for characters used as shapes rather than as text.
    outline_keys: FastMap<char, Option<cosmic_text::CacheKey>>,
    font_sources: HashSet<String>,
    /// Fields already measured, by the glyph they belong to.
    ///
    /// The distance transform is far too slow to run per frame, and it does not
    /// have to be: a glyph's shape does not change, so this is filled once the
    /// first time a letter is drawn and read from thereafter however many sizes
    /// it is later drawn at.
    fields: FastMap<u64, Option<Rc<FieldImage>>>,
}

/// Pixel format of one rasterized glyph image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterContent {
    /// One alpha byte per pixel.
    Mask,
    /// Four RGBA bytes per pixel.
    Color,
    /// One distance byte per pixel, measured at a fixed reference size.
    ///
    /// Shares the mask atlas — it is a single channel either way — but is read
    /// as a distance from the glyph edge rather than as coverage of it, so one
    /// entry draws the letter at any size.
    Field,
}

/// Positioned glyph bitmap ready for atlas upload.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterGlyph {
    /// Process-local key identifying the cached raster image.
    pub cache_key: u64,
    /// Physical left edge relative to the render target.
    ///
    /// Fractional. A glyph measured once and drawn at any size has no reason to
    /// land on a whole pixel, and forcing it onto one is what makes letters sit
    /// unevenly apart — the spacing error is the rounding, accumulated across a
    /// word. Subpixel positioning is what a rasterizer used its subpixel bins
    /// for; a field needs only to be told where to go.
    pub x: f32,
    /// Physical top edge relative to the render target.
    pub y: f32,
    /// Bitmap width, in the pixels the bitmap was measured at.
    pub width: u32,
    /// Bitmap height, in the pixels the bitmap was measured at.
    pub height: u32,
    /// Quad width in physical pixels.
    ///
    /// The same as `width` for anything rasterized at the size it is drawn. A
    /// distance field is measured once at a reference size and then drawn at
    /// whatever size is asked for, so for those two this is the only place the
    /// two numbers part company: the atlas holds `width`, the screen gets this.
    pub draw_width: f32,
    /// Quad height in physical pixels.
    pub draw_height: f32,
    /// Bitmap pixel format.
    pub content: RasterContent,
    /// Tightly packed bitmap bytes.
    ///
    /// Shared rather than owned. The atlas reads these only on a miss — once a
    /// glyph is uploaded, every later frame finds it by key and never looks —
    /// but the bytes were copied out of the cache on every frame regardless,
    /// which for distance fields is a several-kilobyte memcpy per visible glyph
    /// per frame to hand over something nobody reads.
    pub data: Rc<Vec<u8>>,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem {
    /// Loads the system font database and initializes empty caches.
    pub fn new() -> Self {
        let mut fonts = FontSystem::new();
        configure_generic_families(&mut fonts);
        let mut system = Self {
            fonts,
            glyphs: SwashCache::new(),
            buffers: FastMap::default(),
            field_pairs: FastMap::default(),
            outline_keys: FastMap::default(),
            font_sources: HashSet::new(),
            fields: FastMap::default(),
        };
        if let Some(paths) = std::env::var_os("MORF_FONT_PATH") {
            for path in std::env::split_paths(&paths) {
                let _ = system.load_font_path(path);
            }
        }
        system
    }

    /// Loads one font file or every font below a directory.
    pub fn load_font_path(&mut self, path: impl AsRef<Path>) -> io::Result<usize> {
        let path = path.as_ref();
        let before = self.fonts.db().len();
        if path.is_dir() {
            self.fonts.db_mut().load_fonts_dir(path);
        } else {
            self.fonts.db_mut().load_font_file(path)?;
        }
        let loaded = self.fonts.db().len().saturating_sub(before);
        if loaded > 0 {
            configure_generic_families(&mut self.fonts);
            self.buffers.clear();
        }
        Ok(loaded)
    }

    /// Reports whether an exact family name exists in the loaded database.
    pub fn has_family(&self, family: &str) -> bool {
        installed_family(&self.fonts, family).is_some()
    }

    /// Resolves a family stack to an installed family or a generic fallback.
    pub fn resolved_family(&self, family: &str) -> String {
        resolve_family(&self.fonts, family).name().to_owned()
    }

    fn load_font_source(&mut self, source: Option<&str>) {
        let Some(source) = source.filter(|source| !source.is_empty()) else {
            return;
        };
        if self.font_sources.insert(source.to_owned()) {
            let path = source.strip_prefix("file://").unwrap_or(source);
            let _ = self.load_font_path(path);
        }
    }

    /// Returns the shaped buffer retained for a text node.
    pub fn buffer(&self, node: NodeHandle) -> Option<&Buffer> {
        self.buffers
            .get(&BufferKey::own(node))
            .map(|cached| &cached.buffer)
    }

    /// Provides mutable access to the glyph rasterization cache and font database.
    pub fn rasterizer(&mut self) -> (&mut FontSystem, &mut SwashCache) {
        (&mut self.fonts, &mut self.glyphs)
    }

    /// Drops the shaped buffer belonging to a removed scene node.
    pub fn remove(&mut self, node: NodeHandle) {
        self.buffers.remove(&BufferKey::own(node));
        self.buffers.remove(&BufferKey::target(node));
    }
}
fn elided_text(
    fonts: &mut FontSystem,
    text: &str,
    family: &str,
    size: f32,
    options: &TextOptions,
) -> String {
    let Some(width) = options
        .width
        .filter(|_| !options.wrap && options.elide != TextElide::None)
    else {
        return text.to_owned();
    };
    if shaped_width(fonts, text, family, size, options.font_weight) <= width as f32 {
        return text.to_owned();
    }
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut low = 0;
    let mut high = graphemes.len();
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate = elide_candidate(&graphemes, middle, options.elide);
        if shaped_width(fonts, &candidate, family, size, options.font_weight) <= width as f32 {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    elide_candidate(&graphemes, low, options.elide)
}

fn elide_candidate(graphemes: &[&str], kept: usize, mode: TextElide) -> String {
    let kept = kept.min(graphemes.len());
    match mode {
        TextElide::None => graphemes.concat(),
        TextElide::Left => format!("…{}", graphemes[graphemes.len() - kept..].concat()),
        TextElide::Right => format!("{}…", graphemes[..kept].concat()),
        TextElide::Middle => {
            let left = kept.div_ceil(2);
            let right = kept - left;
            format!(
                "{}…{}",
                graphemes[..left].concat(),
                graphemes[graphemes.len() - right..].concat()
            )
        }
    }
}

fn shaped_width(
    fonts: &mut FontSystem,
    text: &str,
    family: &str,
    size: f32,
    font_weight: f64,
) -> f32 {
    let family = resolve_family(fonts, family);
    let mut buffer = Buffer::new(fonts, Metrics::relative(size, 1.2));
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        text,
        &Attrs::new()
            .family(family.family())
            .weight(Weight(normalize_font_weight(font_weight))),
        Shaping::Advanced,
        Some(Align::Left),
    );
    buffer.shape_until_scroll(fonts, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0, f32::max)
}

fn resolve_family(fonts: &FontSystem, requested: &str) -> ResolvedFamily {
    for candidate in requested
        .split(',')
        .map(clean_family)
        .filter(|name| !name.is_empty())
    {
        let generic = match candidate.to_ascii_lowercase().as_str() {
            "serif" => Some(ResolvedFamily::Serif),
            "sans-serif" | "sans serif" | "sans" => Some(ResolvedFamily::SansSerif),
            "monospace" | "mono" => Some(ResolvedFamily::Monospace),
            "cursive" => Some(ResolvedFamily::Cursive),
            "fantasy" => Some(ResolvedFamily::Fantasy),
            _ => None,
        };
        if let Some(generic) = generic {
            return generic;
        }
        if let Some(installed) = installed_family(fonts, candidate) {
            return ResolvedFamily::Name(installed);
        }
    }
    if looks_monospace(requested) {
        ResolvedFamily::Monospace
    } else {
        ResolvedFamily::SansSerif
    }
}

fn clean_family(family: &str) -> &str {
    family
        .trim()
        .trim_matches(|character| character == '\'' || character == '"')
}

fn installed_family(fonts: &FontSystem, requested: &str) -> Option<String> {
    fonts.db().faces().find_map(|face| {
        face.families
            .iter()
            .find(|(family, _)| family.eq_ignore_ascii_case(requested))
            .map(|(family, _)| family.clone())
    })
}

fn looks_monospace(family: &str) -> bool {
    let family = family.to_ascii_lowercase();
    family.contains("mono")
        || family.contains("iosevka")
        || family.contains("terminal")
        || family.contains("typewriter")
        || family.contains("code")
}

fn configure_generic_families(fonts: &mut FontSystem) {
    let sans = preferred_family(
        fonts,
        &[
            "Noto Sans",
            "DejaVu Sans",
            "Liberation Sans",
            "Cantarell",
            "Nimbus Sans",
        ],
        |monospaced| !monospaced,
    );
    let serif = preferred_family(
        fonts,
        &[
            "Noto Serif",
            "DejaVu Serif",
            "Liberation Serif",
            "Nimbus Roman",
        ],
        |monospaced| !monospaced,
    );
    let monospace = preferred_family(
        fonts,
        &[
            "Noto Sans Mono",
            "DejaVu Sans Mono",
            "Liberation Mono",
            "Nimbus Mono PS",
        ],
        |monospaced| monospaced,
    );
    let db = fonts.db_mut();
    if let Some(family) = sans {
        db.set_sans_serif_family(family);
    }
    if let Some(family) = serif {
        db.set_serif_family(family);
    }
    if let Some(family) = monospace {
        db.set_monospace_family(family);
    }
}

fn preferred_family(
    fonts: &FontSystem,
    preferred: &[&str],
    fallback: impl Fn(bool) -> bool,
) -> Option<String> {
    preferred
        .iter()
        .find_map(|family| installed_family(fonts, family))
        .or_else(|| {
            fonts.db().faces().find_map(|face| {
                fallback(face.monospaced)
                    .then(|| face.families.first().map(|(family, _)| family.clone()))
                    .flatten()
            })
        })
}

fn normalize_font_weight(weight: f64) -> u16 {
    if weight.is_finite() {
        weight.round().clamp(100.0, 900.0) as u16
    } else {
        400
    }
}

mod glyph_fields;
mod glyph_morph;
pub use glyph_morph::CONTOUR_POINTS as GLYPH_CONTOUR_POINTS;
mod glyph_runs;
mod measure;
mod raster_glyph;

pub use glyph_fields::{
    FIELD_REFERENCE_PX as GLYPH_FIELD_REFERENCE_PX, FIELD_SPREAD_PX as GLYPH_FIELD_SPREAD_PX,
};

#[cfg(test)]
mod tests;
