use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;

use super::*;

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

include!("config.rs");
include!("core_api.rs");
include!("modules.rs");
include!("input_api.rs");
include!("scene.rs");
include!("lifecycle_io.rs");
include!("events_animation.rs");
include!("animation_playback.rs");
include!("animation_groups.rs");
include!("views_states.rs");
include!("layer_surfaces.rs");
include!("screens.rs");
include!("examples.rs");
