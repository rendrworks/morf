//! Text shaping, measurement, and glyph rasterization for mold.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Wrap,
};
use mold_layout::{Size, TextAlignment, TextElide, TextMeasurer, TextOptions};
use mold_scene::NodeHandle;
use unicode_segmentation::UnicodeSegmentation;

struct CachedBuffer {
    buffer: Buffer,
    input: Option<TextInput>,
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
}

/// Shared font database, per-node shaped buffers, and glyph image cache.
pub struct TextSystem {
    fonts: FontSystem,
    glyphs: SwashCache,
    buffers: HashMap<NodeHandle, CachedBuffer>,
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
        Self {
            fonts: FontSystem::new(),
            glyphs: SwashCache::new(),
            buffers: HashMap::new(),
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
        let size = size.max(1.0) as f32;
        let input = TextInput {
            text: text.to_owned(),
            family: family.to_owned(),
            size: (size as f64).to_bits(),
            width: options.width.map(f64::to_bits),
            wrap: options.wrap,
            alignment: options.alignment,
            elide: options.elide,
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
            let displayed = elided_text(&mut self.fonts, text, family, size, options);
            cached.buffer.set_text(
                &displayed,
                &Attrs::new().family(Family::Name(family)),
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
    options: TextOptions,
) -> String {
    let Some(width) = options
        .width
        .filter(|_| !options.wrap && options.elide != TextElide::None)
    else {
        return text.to_owned();
    };
    if shaped_width(fonts, text, family, size) <= width as f32 {
        return text.to_owned();
    }
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut low = 0;
    let mut high = graphemes.len();
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate = elide_candidate(&graphemes, middle, options.elide);
        if shaped_width(fonts, &candidate, family, size) <= width as f32 {
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

fn shaped_width(fonts: &mut FontSystem, text: &str, family: &str, size: f32) -> f32 {
    let mut buffer = Buffer::new(fonts, Metrics::relative(size, 1.2));
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        text,
        &Attrs::new().family(Family::Name(family)),
        Shaping::Advanced,
        Some(Align::Left),
    );
    buffer.shape_until_scroll(fonts, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
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
                TextOptions {
                    width: Some(100.0),
                    elide: mode,
                    ..TextOptions::default()
                },
            );
            assert!(displayed.contains('…'));
            assert!(shaped_width(&mut fonts, &displayed, "sans-serif", 16.0) <= 100.0);
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
