use std::collections::BTreeMap;

use morf_scene::{Behavior, Element, NodeHandle, Scene, Value};

use crate::*;

struct FixedText;

impl TextMeasurer for FixedText {
    fn measure(
        &mut self,
        _node: NodeHandle,
        text: &str,
        _family: &str,
        size: f64,
        _options: TextOptions,
    ) -> Size {
        Size {
            width: text.len() as f64 * size / 2.0,
            height: size,
        }
    }

    fn measure_image(
        &mut self,
        _node: NodeHandle,
        element: Element,
        source: &str,
        _theme: Option<&str>,
    ) -> Option<Size> {
        (element == Element::Image && !source.is_empty()).then_some(Size {
            width: 64.0,
            height: 32.0,
        })
    }
}

/// Measures like `FixedText`, but wraps: given a width, the text folds into
/// as many lines as it needs.
struct WrapText;

impl TextMeasurer for WrapText {
    fn measure(
        &mut self,
        _node: NodeHandle,
        text: &str,
        _family: &str,
        size: f64,
        options: TextOptions,
    ) -> Size {
        let full = text.len() as f64 * size / 2.0;
        match options.width.filter(|_| options.wrap) {
            Some(width) if width > 0.0 => Size {
                width: full.min(width),
                height: (full / width).ceil().max(1.0) * size,
            },
            _ => Size {
                width: full,
                height: size,
            },
        }
    }
}

struct WeightText(f64);

impl TextMeasurer for WeightText {
    fn measure(
        &mut self,
        _node: NodeHandle,
        _text: &str,
        _family: &str,
        _size: f64,
        options: TextOptions,
    ) -> Size {
        self.0 = options.font_weight;
        Size::default()
    }
}

mod alignment;
mod basic;
mod custom;
mod flex;
mod transforms;
mod views;
