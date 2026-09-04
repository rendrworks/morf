mod arguments;
mod colors;
use morf_scene::NodeHandle;

use crate::*;

struct NoText;

impl morf_layout::TextMeasurer for NoText {
    fn measure(
        &mut self,
        _node: NodeHandle,
        _text: &str,
        _family: &str,
        _size: f64,
        _options: morf_layout::TextOptions,
    ) -> morf_layout::Size {
        morf_layout::Size::default()
    }
}

mod animation_groups;
mod animation_playback;
mod config;
mod core_api;
mod diagnostics;
mod events_animation;
mod examples;
mod flushing;
mod gradients;
mod idle_input;
mod input_api;
mod layer_surfaces;
mod lifecycle_io;
mod modules;
mod pam_session;
mod prefers;
mod sandbox_limits;
mod scene;
mod screens;
mod services;
mod shaders;
mod state_tables;
mod themes;
mod views_states;
