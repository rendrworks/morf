//! Text shaping, measurement, and glyph rasterization for mold.

use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use mold_layout::{Size, TextMeasurer};
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
    wrap_width: Option<u64>,
}

/// Shared font database, per-node shaped buffers, and glyph image cache.
pub struct TextSystem {
    fonts: FontSystem,
    glyphs: SwashCache,
    buffers: HashMap<NodeHandle, CachedBuffer>,
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
}

impl TextMeasurer for TextSystem {
    fn measure(
        &mut self,
        node: NodeHandle,
        text: &str,
        family: &str,
        size: f64,
        wrap_width: Option<f64>,
    ) -> Size {
        let size = size.max(1.0) as f32;
        let input = TextInput {
            text: text.to_owned(),
            family: family.to_owned(),
            size: (size as f64).to_bits(),
            wrap_width: wrap_width.map(f64::to_bits),
        };
        let cached = self.buffers.entry(node).or_insert_with(|| CachedBuffer {
            buffer: Buffer::new(&mut self.fonts, Metrics::relative(size, 1.2)),
            input: None,
        });
        if cached.input.as_ref() != Some(&input) {
            cached.buffer.set_metrics_and_size(
                Metrics::relative(size, 1.2),
                wrap_width.map(|v| v as f32),
                None,
            );
            cached.buffer.set_wrap(if wrap_width.is_some() {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            cached.buffer.set_text(
                text,
                &Attrs::new().family(Family::Name(family)),
                Shaping::Advanced,
                None,
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

        let measured = text.measure(node, "mold", "sans-serif", 16.0, None);

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

        let unwrapped = text.measure(unwrapped_node, content, "sans-serif", 16.0, None);
        let wrapped = text.measure(wrapped_node, content, "sans-serif", 16.0, Some(80.0));

        assert!(wrapped.width <= 80.0);
        assert!(wrapped.height > unwrapped.height);
    }
}
