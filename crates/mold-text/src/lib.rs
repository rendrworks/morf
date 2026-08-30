//! Text shaping, measurement, and glyph rasterization for mold.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::Path;

use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight,
    Wrap,
};
use mold_layout::{Size, TextAlignment, TextElide, TextMeasurer, TextOptions};
use mold_scene::NodeHandle;
use unicode_segmentation::UnicodeSegmentation;

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

/// Shared font database, per-node shaped buffers, and glyph image cache.
pub struct TextSystem {
    fonts: FontSystem,
    glyphs: SwashCache,
    buffers: HashMap<NodeHandle, CachedBuffer>,
    font_sources: HashSet<String>,
}

/// Pixel format of one rasterized glyph image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterContent {
    /// One alpha byte per pixel.
    Mask,
    /// Four RGBA bytes per pixel.
    Color,
}

/// Positioned glyph bitmap ready for atlas upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RasterGlyph {
    /// Process-local key identifying the cached raster image.
    pub cache_key: u64,
    /// Physical left edge relative to the render target.
    pub x: i32,
    /// Physical top edge relative to the render target.
    pub y: i32,
    /// Bitmap width in physical pixels.
    pub width: u32,
    /// Bitmap height in physical pixels.
    pub height: u32,
    /// Bitmap pixel format.
    pub content: RasterContent,
    /// Tightly packed bitmap bytes.
    pub data: Vec<u8>,
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
            buffers: HashMap::new(),
            font_sources: HashSet::new(),
        };
        if let Some(paths) = std::env::var_os("MOLD_FONT_PATH") {
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
        self.buffers.get(&node).map(|cached| &cached.buffer)
    }

    /// Provides mutable access to the glyph rasterization cache and font database.
    pub fn rasterizer(&mut self) -> (&mut FontSystem, &mut SwashCache) {
        (&mut self.fonts, &mut self.glyphs)
    }

    /// Drops the shaped buffer belonging to a removed scene node.
    pub fn remove(&mut self, node: NodeHandle) {
        self.buffers.remove(&node);
    }

    /// Rasterizes one cached text node at a physical origin and scale.
    pub fn rasterize(
        &mut self,
        node: NodeHandle,
        origin: (f32, f32),
        scale: f32,
    ) -> Vec<RasterGlyph> {
        let Some(buffer) = self.buffers.get(&node) else {
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
            .filter_map(|glyph| {
                let mut hasher = DefaultHasher::new();
                glyph.cache_key.hash(&mut hasher);
                let cache_key = hasher.finish();
                let image = self
                    .glyphs
                    .get_image(&mut self.fonts, glyph.cache_key)
                    .clone()?;
                let content = match image.content {
                    SwashContent::Mask => RasterContent::Mask,
                    SwashContent::Color => RasterContent::Color,
                    SwashContent::SubpixelMask => RasterContent::Mask,
                };
                Some(RasterGlyph {
                    cache_key,
                    x: glyph.x + image.placement.left,
                    y: glyph.y - image.placement.top,
                    width: image.placement.width,
                    height: image.placement.height,
                    content,
                    data: image.data,
                })
            })
            .collect()
    }
}

impl TextMeasurer for TextSystem {
    fn measure(
        &mut self,
        node: NodeHandle,
        text: &str,
        family: &str,
        size: f64,
        options: TextOptions,
    ) -> Size {
        self.load_font_source(options.font_source.as_deref());
        let size = size.max(1.0) as f32;
        let font_weight = normalize_font_weight(options.font_weight);
        let input = TextInput {
            text: text.to_owned(),
            family: family.to_owned(),
            size: (size as f64).to_bits(),
            width: options.width.map(f64::to_bits),
            wrap: options.wrap,
            alignment: options.alignment,
            elide: options.elide,
            font_weight,
            font_source: options.font_source.clone(),
        };
        let cached = self.buffers.entry(node).or_insert_with(|| CachedBuffer {
            buffer: Buffer::new(&mut self.fonts, Metrics::relative(size, 1.2)),
            input: None,
        });
        if cached.input.as_ref() != Some(&input) {
            cached.buffer.set_metrics_and_size(
                Metrics::relative(size, 1.2),
                options.width.map(|value| value as f32),
                None,
            );
            cached.buffer.set_wrap(if options.wrap {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            let displayed = elided_text(&mut self.fonts, text, family, size, &options);
            let family = resolve_family(&self.fonts, family);
            cached.buffer.set_text(
                &displayed,
                &Attrs::new()
                    .family(family.family())
                    .weight(Weight(font_weight)),
                Shaping::Advanced,
                Some(match options.alignment {
                    TextAlignment::Left => Align::Left,
                    TextAlignment::Right => Align::Right,
                    TextAlignment::Center => Align::Center,
                    TextAlignment::Justified => Align::Justified,
                }),
            );
            cached.buffer.shape_until_scroll(&mut self.fonts, false);
            cached.input = Some(input);
        }

        let mut width = 0.0_f32;
        let mut height = 0.0_f32;
        for run in cached.buffer.layout_runs() {
            width = width.max(run.line_w);
            height = height.max(run.line_top + run.line_height);
        }
        Size {
            width: width as f64,
            height: height as f64,
        }
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

#[cfg(test)]
mod tests {
    use cosmic_text::fontdb::Source;
    use mold_scene::{Element, Scene};

    use super::*;

    #[test]
    fn shapes_and_caches_a_buffer_per_text_node() {
        let mut scene = Scene::new();
        let node = scene.create(Element::Text);
        let mut text = TextSystem::new();

        let measured = text.measure(node, "mold", "sans-serif", 16.0, TextOptions::default());

        assert!(measured.width > 0.0);
        assert!(measured.height > 0.0);
        assert!(text.buffer(node).is_some());
    }

    #[test]
    fn font_weight_participates_in_the_shaping_cache() {
        let mut scene = Scene::new();
        let node = scene.create(Element::Text);
        let mut text = TextSystem::new();

        text.measure(
            node,
            "mold",
            "sans-serif",
            16.0,
            TextOptions {
                font_weight: 700.0,
                ..TextOptions::default()
            },
        );

        assert_eq!(text.buffers[&node].input.as_ref().unwrap().font_weight, 700);
        assert_eq!(normalize_font_weight(50.0), 100);
        assert_eq!(normalize_font_weight(950.0), 900);
        assert_eq!(normalize_font_weight(f64::NAN), 400);
    }

    #[test]
    fn missing_mono_family_keeps_monospace_advances() {
        let mut scene = Scene::new();
        let narrow = scene.create(Element::Text);
        let wide = scene.create(Element::Text);
        let mut text = TextSystem::new();

        let narrow = text.measure(
            narrow,
            "iiii",
            "Unavailable Nerd Font Mono",
            16.0,
            TextOptions::default(),
        );
        let wide = text.measure(
            wide,
            "WWWW",
            "Unavailable Nerd Font Mono",
            16.0,
            TextOptions::default(),
        );

        assert_eq!(
            text.resolved_family("Unavailable Nerd Font Mono"),
            "monospace"
        );
        assert!((narrow.width - wide.width).abs() < 0.01);
    }

    #[test]
    fn family_stack_uses_an_installed_fallback() {
        let text = TextSystem::new();
        let installed = text
            .fonts
            .db()
            .faces()
            .find_map(|face| face.families.first())
            .map(|(family, _)| family.clone())
            .expect("system font database should not be empty");
        let request = format!("Missing Family, '{installed}'");

        assert_eq!(text.resolved_family(&request), installed);
        assert!(text.has_family(&installed.to_ascii_uppercase()));
    }

    #[test]
    fn missing_font_path_reports_an_error() {
        let mut text = TextSystem::new();
        let error = text
            .load_font_path("/mold-test-font-does-not-exist.ttf")
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn text_font_source_loads_once_before_shaping() {
        let mut scene = Scene::new();
        let node = scene.create(Element::Text);
        let mut text = TextSystem::new();
        let (source, family) = text
            .fonts
            .db()
            .faces()
            .find_map(|face| match &face.source {
                Source::File(path) => face
                    .families
                    .first()
                    .map(|(family, _)| (path.clone(), family.clone())),
                _ => None,
            })
            .expect("system database should contain a file-backed font");
        let source = format!("file://{}", source.display());
        let before = text.fonts.db().len();
        let options = TextOptions {
            font_source: Some(source.clone()),
            ..TextOptions::default()
        };

        text.measure(node, "mold", &family, 16.0, options.clone());
        let loaded = text.fonts.db().len();
        text.measure(node, "mold", &family, 16.0, options);

        assert!(loaded > before);
        assert_eq!(text.fonts.db().len(), loaded);
        assert_eq!(text.font_sources.len(), 1);
        assert_eq!(
            text.buffers[&node].input.as_ref().unwrap().font_source,
            Some(source)
        );
    }

    #[test]
    fn wrapping_constrains_width_and_increases_height() {
        let mut scene = Scene::new();
        let unwrapped_node = scene.create(Element::Text);
        let wrapped_node = scene.create(Element::Text);
        let mut text = TextSystem::new();
        let content = "a shell runtime configured entirely in Lua";

        let unwrapped = text.measure(
            unwrapped_node,
            content,
            "sans-serif",
            16.0,
            TextOptions {
                width: Some(80.0),
                wrap: false,
                ..TextOptions::default()
            },
        );
        let wrapped = text.measure(
            wrapped_node,
            content,
            "sans-serif",
            16.0,
            TextOptions {
                width: Some(80.0),
                wrap: true,
                ..TextOptions::default()
            },
        );

        assert!(wrapped.width <= 80.0);
        assert!(wrapped.height > unwrapped.height);
        assert!(unwrapped.width > 80.0);
    }

    #[test]
    fn centered_text_offsets_glyphs_inside_width() {
        let mut scene = Scene::new();
        let node = scene.create(Element::Text);
        let mut text = TextSystem::new();
        text.measure(
            node,
            "mold",
            "sans-serif",
            16.0,
            TextOptions {
                width: Some(200.0),
                alignment: TextAlignment::Center,
                ..TextOptions::default()
            },
        );

        let first_x = text
            .buffer(node)
            .unwrap()
            .layout_runs()
            .next()
            .unwrap()
            .glyphs[0]
            .x;
        assert!(first_x > 0.0);
    }

    #[test]
    fn rasterizes_cached_text_at_fractional_scale() {
        let mut scene = Scene::new();
        let node = scene.create(Element::Text);
        let mut text = TextSystem::new();
        text.measure(node, "mold", "sans-serif", 16.0, TextOptions::default());

        let glyphs = text.rasterize(node, (5.0, 7.0), 1.25);
        let cached = text.rasterize(node, (5.0, 7.0), 1.25);

        assert!(!glyphs.is_empty());
        assert!(
            glyphs
                .iter()
                .all(|glyph| glyph.width > 0 && glyph.height > 0)
        );
        assert!(glyphs.iter().all(|glyph| !glyph.data.is_empty()));
        assert_eq!(
            glyphs
                .iter()
                .map(|glyph| glyph.cache_key)
                .collect::<Vec<_>>(),
            cached
                .iter()
                .map(|glyph| glyph.cache_key)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn eliding_places_ellipsis_and_constrains_width() {
        let mut fonts = FontSystem::new();
        let text = "application launcher settings";
        for mode in [TextElide::Left, TextElide::Middle, TextElide::Right] {
            let displayed = elided_text(
                &mut fonts,
                text,
                "sans-serif",
                16.0,
                &TextOptions {
                    width: Some(100.0),
                    elide: mode,
                    ..TextOptions::default()
                },
            );
            assert!(displayed.contains('…'));
            assert!(shaped_width(&mut fonts, &displayed, "sans-serif", 16.0, 400.0) <= 100.0);
            match mode {
                TextElide::Left => assert!(text.ends_with(displayed.trim_start_matches('…'))),
                TextElide::Middle => {
                    let (left, right) = displayed.split_once('…').unwrap();
                    assert!(text.starts_with(left));
                    assert!(text.ends_with(right));
                }
                TextElide::Right => assert!(text.starts_with(displayed.trim_end_matches('…'))),
                TextElide::None => unreachable!(),
            }
        }
    }
}
