//! The backdrop: how the shell hears a click anywhere else on the screen.
//!
//! A shell whose surface is only as big as what it shows cannot see a click
//! beside it, and the usual answer — a surface covering the screen — makes
//! every frame a screen-sized frame. The backdrop is a second layer surface
//! that covers the output but is never painted: one transparent shm pixel
//! the compositor stretches, so it costs no GPU time, no swapchain, and no
//! synchronisation. Its input region is the whole output minus the shell's
//! own surface while `morf.surface.backdrop` is true, and empty otherwise,
//! so it is inert until a page opens. A press on it reaches the
//! configuration as `morf.on_backdrop_click`.
//!
//! It is created when the configuration declares `morf.surface.backdrop` at
//! all, since a layer surface's place in its layer is fixed at creation.

use morf_lua::LayerSurfaceConfig;
use morf_wayland::{InputRect, KeyboardFocus, LayerAnchors, LayerClient};

use crate::surfaces::*;

/// Wayland identifier of the backdrop surface.
pub(crate) const BACKDROP_LAYER: u64 = u64::MAX - 4;

/// Opens the backdrop under the shell's surface, inert, when it is declared.
pub(crate) fn open_backdrop_layer(
    client: &mut LayerClient,
    config: &LayerSurfaceConfig,
    output: &str,
) -> Result<(), String> {
    eprintln!("backdrop declared: {:?}", config.backdrop);
    if config.backdrop.is_none() {
        return Ok(());
    }
    let mut bar = runtime_bar_config(config, output)?;
    bar.namespace = format!("{}-backdrop", config.namespace);
    bar.width = 0;
    bar.height = 0;
    bar.exclusive_zone = -1;
    bar.anchors = LayerAnchors {
        top: true,
        right: true,
        bottom: true,
        left: true,
    };
    bar.margin_top = 0;
    bar.margin_right = 0;
    bar.margin_bottom = 0;
    bar.margin_left = 0;
    bar.keyboard_focus = KeyboardFocus::None;
    client
        .open_layer(BACKDROP_LAYER, bar)
        .map_err(|error| error.to_string())?;
    client.set_layer_input_region(BACKDROP_LAYER, Some(&[]));
    client
        .map_layer_blank(BACKDROP_LAYER)
        .map_err(|error| error.to_string())?;
    client.commit_layer(BACKDROP_LAYER);
    Ok(())
}

/// Where the shell's own surface sits on its output, in logical pixels: the
/// hole in the backdrop's input region, so the shell keeps its own clicks.
fn primary_rect(client: &LayerClient, config: &LayerSurfaceConfig, out: (i32, i32)) -> InputRect {
    let (width, height) = client.logical_size();
    let (width, height) = (width as i32, height as i32);
    let anchors = &config.anchors;
    let x = if anchors.left {
        config.margin_left
    } else if anchors.right {
        out.0 - width - config.margin_right
    } else {
        (out.0 - width) / 2
    };
    let y = if anchors.top {
        config.margin_top
    } else if anchors.bottom {
        out.1 - height - config.margin_bottom
    } else {
        (out.1 - height) / 2
    };
    InputRect {
        x,
        y,
        width,
        height,
    }
}

/// Brings the backdrop's input region up to date with `morf.surface.backdrop`.
pub(crate) fn apply_backdrop(client: &LayerClient, config: &LayerSurfaceConfig, output: &str) {
    if client.layer_surface(BACKDROP_LAYER).is_none() {
        return;
    }
    let screen = client
        .screens()
        .iter()
        .find(|screen| screen.name.as_deref() == Some(output))
        .and_then(|screen| screen.size);
    let regions = match (config.backdrop, screen) {
        (Some(true), Some(out)) => {
            let hole = primary_rect(client, config, out);
            let below = hole.y + hole.height;
            let right = hole.x + hole.width;
            vec![
                InputRect { x: 0, y: 0, width: out.0, height: hole.y },
                InputRect { x: 0, y: below, width: out.0, height: out.1 - below },
                InputRect { x: 0, y: hole.y, width: hole.x, height: hole.height },
                InputRect { x: right, y: hole.y, width: out.0 - right, height: hole.height },
            ]
        }
        _ => Vec::new(),
    };
    client.set_layer_input_region(BACKDROP_LAYER, Some(&regions));
    client.commit_layer(BACKDROP_LAYER);
}
