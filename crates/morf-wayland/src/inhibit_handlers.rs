//! Things the shell asks the compositor *not* to do.
//!
//! Idle inhibition and shortcut inhibition are the same shape: an object whose
//! existence is the request, destroyed to withdraw it. Split from
//! `protocol_handlers` at the line gate, and a fair seam -- everything there
//! describes a surface; these describe a favour asked of the compositor.

use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
    zwp_keyboard_shortcuts_inhibitor_v1::{self, ZwpKeyboardShortcutsInhibitorV1},
};

use crate::{state_types::LayerState, surface_types::LayerEvent};

// Neither half of idle inhibition says anything back: the manager only makes
// inhibitors, and an inhibitor is a token whose existence is the whole message.
wayland_client::delegate_noop!(LayerState: ignore ZwpIdleInhibitManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpIdleInhibitorV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpKeyboardShortcutsInhibitManagerV1);

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for LayerState {
    /// Whether the compositor is actually honouring the request.
    ///
    /// Asking is not getting: a compositor may refuse, or grant and later
    /// withdraw when focus moves. A shell that assumed its keys were its own
    /// would draw a launcher and then watch Super open the compositor's.
    fn event(
        state: &mut Self,
        _inhibitor: &ZwpKeyboardShortcutsInhibitorV1,
        event: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let active = match event {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => true,
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => false,
            _ => return,
        };
        state
            .events
            .push_back(LayerEvent::ShortcutsInhibited { active });
    }
}
