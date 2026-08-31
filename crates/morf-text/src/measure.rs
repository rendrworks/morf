// What the layout engine asks of text: how big a string comes out, given a
// font, a size and the room it has.

use cosmic_text::{Align, Attrs, Buffer, Metrics, Shaping, Weight, Wrap};
use morf_layout::{Size, TextAlignment, TextMeasurer, TextOptions};
use morf_scene::NodeHandle;

use crate::{
    CachedBuffer, TextInput, TextSystem, elided_text, normalize_font_weight, resolve_family,
};

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
