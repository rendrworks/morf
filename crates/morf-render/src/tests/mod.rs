use std::convert::Infallible;

use morf_layout::{Geometry, Layout, Size, TextAlignment, TextElide, TextMeasurer, Transform2D};
use morf_scene::{Color, Element, NodeHandle, Scene, Value};

use crate::commands::*;
use crate::damage::*;
use crate::effects::*;
use crate::field::*;

#[test]
fn scene_srgb_colors_are_linearized_for_gpu_output() {
    let color = Color::rgba8(0x46, 0x48, 0x58, 0x80);
    let converted = color_array(color);

    assert!((converted[0] - 0.061_246_07).abs() < 1e-7);
    assert!((converted[1] - 0.064_803_27).abs() < 1e-7);
    assert!((converted[2] - 0.097_587_35).abs() < 1e-7);
    assert_eq!(converted[3], color.alpha);
    assert_eq!(srgb_channel_to_linear(0.0), 0.0);
    assert_eq!(srgb_channel_to_linear(1.0), 1.0);
}

pub(crate) struct NoText;

impl TextMeasurer for NoText {
    fn measure(
        &mut self,
        _node: NodeHandle,
        _text: &str,
        _family: &str,
        _size: f64,
        _options: morf_layout::TextOptions,
    ) -> Size {
        Size::default()
    }
}

#[derive(Default)]
struct RecordingBackend {
    frames: usize,
    damage: Vec<DamageRect>,
    size: (u32, u32),
}

impl RenderBackend for RecordingBackend {
    type Error = Infallible;

    fn render(
        &mut self,
        _list: &DrawList,
        damage: &[DamageRect],
        _scale_120: u32,
    ) -> Result<(), Self::Error> {
        self.frames += 1;
        self.damage = damage.to_vec();
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
    }
}

mod damage;
mod field_packing;
mod fields;
mod outline_boxes;
mod transform_text;
mod tree;
