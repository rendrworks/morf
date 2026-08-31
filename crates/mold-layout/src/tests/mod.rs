use std::collections::BTreeMap;

use mold_scene::{Behavior, Element, NodeHandle, Scene, Value};

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

mod basic;
mod transforms;
mod views;
