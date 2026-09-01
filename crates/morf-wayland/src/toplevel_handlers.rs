//! The compositor's window list.
//!
//! Split from `protocol_handlers` at the line gate, which is a fair seam: every
//! other protocol there describes *this* client's own surfaces, and these two
//! describe everybody else's windows. That is a different kind of thing to know
//! about, and the only one a configuration can watch without having asked for
//! it.

use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};

use crate::{state_types::LayerState, types::ToplevelInfo};

impl Dispatch<ExtForeignToplevelListV1, ()> for LayerState {
    /// A window appeared, or the compositor stopped telling us about them.
    ///
    /// The handle arrives bare: no title, no application, no name. Those follow
    /// as separate events and are only worth reading once `done` says the
    /// description is complete, which is why the entry starts empty here rather
    /// than being invented.
    fn event(
        state: &mut Self,
        _list: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state
                .toplevels
                .insert(toplevel.id(), ToplevelInfo::default());
        }
    }

    wayland_client::event_created_child!(LayerState, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for LayerState {
    /// One window describing itself.
    ///
    /// `title`, `app_id` and `identifier` each arrive on their own, then `done`
    /// says the set is consistent — so nothing is published until then, and a
    /// caller never sees a window with half a description. `closed` is the end
    /// of the window, not of the handle: the entry goes, and the handle is
    /// destroyed so the compositor can forget it too.
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        let id = handle.id();
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                state.toplevels.entry(id).or_default().title = title;
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.toplevels.entry(id).or_default().app_id = app_id;
            }
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                state.toplevels.entry(id).or_default().identifier = identifier;
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {
                state.toplevels_changed = true;
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.remove(&id);
                state.toplevels_changed = true;
                handle.destroy();
            }
            _ => {}
        }
    }
}
