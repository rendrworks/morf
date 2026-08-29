//! Text shaping, measurement, and glyph rasterization for mold.

use std::collections::HashMap;

use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Wrap,
};
use mold_layout::{Size, TextAlignment, TextMeasurer, TextOptions};
use mold_scene::NodeHandle;

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
            cached.buffer.set_text(
                text,
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

        assert!(!glyphs.is_empty());
        assert!(
            glyphs
                .iter()
                .all(|glyph| glyph.width > 0 && glyph.height > 0)
        );
        assert!(glyphs.iter().all(|glyph| !glyph.data.is_empty()));
    }
}
