use mold_scene::NodeHandle;

use crate::*;

struct NoText;

impl mold_layout::TextMeasurer for NoText {
    fn measure(
        &mut self,
        _node: NodeHandle,
        _text: &str,
        _family: &str,
        _size: f64,
        _options: mold_layout::TextOptions,
    ) -> mold_layout::Size {
        mold_layout::Size::default()
    }
}

mod animation_groups;
mod animation_playback;
mod config;
mod core_api;
mod events_animation;
mod examples;
mod input_api;
mod layer_surfaces;
mod lifecycle_io;
mod modules;
mod sandbox_limits;
mod scene;
mod screens;
mod views_states;
