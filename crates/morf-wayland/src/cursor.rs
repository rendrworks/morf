//! The pointer's shape while it is over one of this client's surfaces.
//!
//! Through `wp_cursor_shape_v1`: the compositor draws its own cursor theme,
//! and the client only says which shape it wants. The names are the
//! protocol's own, spelled the way every other morf name is.

use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;

use wayland_client::Proxy;

use crate::{state_types::*, surface_types::*};

wayland_client::delegate_noop!(LayerState: ignore WpCursorShapeManagerV1);
wayland_client::delegate_noop!(LayerState: ignore WpCursorShapeDeviceV1);

/// The protocol shape a name means, and the protocol version it needs.
pub fn cursor_shape(name: &str) -> Option<(Shape, u32)> {
    let shape = match name {
        "default" => Shape::Default,
        "context_menu" => Shape::ContextMenu,
        "help" => Shape::Help,
        "pointer" => Shape::Pointer,
        "progress" => Shape::Progress,
        "wait" => Shape::Wait,
        "cell" => Shape::Cell,
        "crosshair" => Shape::Crosshair,
        "text" => Shape::Text,
        "vertical_text" => Shape::VerticalText,
        "alias" => Shape::Alias,
        "copy" => Shape::Copy,
        "move" => Shape::Move,
        "no_drop" => Shape::NoDrop,
        "not_allowed" => Shape::NotAllowed,
        "grab" => Shape::Grab,
        "grabbing" => Shape::Grabbing,
        "e_resize" => Shape::EResize,
        "n_resize" => Shape::NResize,
        "ne_resize" => Shape::NeResize,
        "nw_resize" => Shape::NwResize,
        "s_resize" => Shape::SResize,
        "se_resize" => Shape::SeResize,
        "sw_resize" => Shape::SwResize,
        "w_resize" => Shape::WResize,
        "ew_resize" => Shape::EwResize,
        "ns_resize" => Shape::NsResize,
        "nesw_resize" => Shape::NeswResize,
        "nwse_resize" => Shape::NwseResize,
        "col_resize" => Shape::ColResize,
        "row_resize" => Shape::RowResize,
        "all_scroll" => Shape::AllScroll,
        "zoom_in" => Shape::ZoomIn,
        "zoom_out" => Shape::ZoomOut,
        "dnd_ask" => return Some((Shape::DndAsk, 2)),
        "all_resize" => return Some((Shape::AllResize, 2)),
        _ => return None,
    };
    Some((shape, 1))
}

impl LayerClient {
    /// Asks for the pointer to take a shape while it is over this client.
    ///
    /// False when the compositor has no cursor-shape protocol, the pointer is
    /// not over one of these surfaces, or the name is not a shape. The same
    /// shape asked for twice is sent once.
    pub fn set_cursor_shape(&mut self, name: &str) -> bool {
        let Some((shape, needs)) = cursor_shape(name) else {
            return false;
        };
        let state = &mut self.state;
        let (Some(manager), Some(pointer), Some(serial)) = (
            &state.cursor_shape_manager,
            &state.pointer,
            state.pointer_enter_serial,
        ) else {
            return false;
        };
        if manager.version() < needs {
            return false;
        }
        if state.cursor_shape_current.as_deref() == Some(name) {
            return true;
        }
        let device = state
            .cursor_device
            .get_or_insert_with(|| manager.get_pointer(pointer, &self.queue.handle(), ()));
        device.set_shape(serial, shape);
        state.cursor_shape_current = Some(name.to_owned());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_the_scene_accepts_is_one_the_protocol_has() {
        for name in morf_scene::CURSOR_SHAPES {
            assert!(cursor_shape(name).is_some(), "{name}");
        }
        assert!(cursor_shape("hand").is_none());
        assert_eq!(cursor_shape("dnd_ask").map(|(_, version)| version), Some(2));
    }
}
