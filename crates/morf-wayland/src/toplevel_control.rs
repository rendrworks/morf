//! Doing something about another window, rather than only listing it.
//!
//! `ext-foreign-toplevel-list` — bound beside this — is enumeration and nothing
//! else. It is the newer protocol and the better one for *knowing* what is
//! open, and by design it has no activate, no close, no state at all. So a
//! shell built on it alone can draw a task list and cannot make it a task
//! *bar*: every entry is a label nobody can click.
//!
//! `wlr-foreign-toplevel-management` is the older protocol that can. Binding
//! both is a deliberate trade: identity and capture stay with the ext list,
//! which is what the capture protocols key on, and control comes from here.
//!
//! The seam between them is the awkward part and worth being honest about.
//! Nothing in either protocol correlates a handle in one with a handle in the
//! other — that is precisely the hole `hyprland-toplevel-mapping` exists to
//! fill, and taking a per-compositor protocol to do it is the thing this engine
//! does not do. So the two are matched on application and title, which is
//! right whenever those differ between windows and ambiguous when they do not.
//! A second window of one application showing the same title will act on
//! whichever matched first.

use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, State, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

use crate::state_types::LayerState;

/// One window as the control protocol describes it.
///
/// Its own record rather than fields on `ToplevelInfo`, because this half is
/// optional: a compositor may offer the list and not the control, and a
/// configuration is entitled to know the difference between a window that is
/// not maximized and one whose compositor never said.
#[derive(Clone, Debug, Default)]
pub(crate) struct ToplevelControl {
    pub(crate) title: String,
    pub(crate) app_id: String,
    pub(crate) activated: bool,
    pub(crate) maximized: bool,
    pub(crate) minimized: bool,
    pub(crate) fullscreen: bool,
    /// Set while the description is incomplete. The protocol sends title,
    /// app_id and state separately and then `done`; publishing in between shows
    /// a window that is briefly neither maximized nor not.
    pub(crate) pending: ToplevelPending,
}

/// The half-built description, before `done` makes it true.
#[derive(Clone, Debug, Default)]
pub(crate) struct ToplevelPending {
    pub(crate) title: String,
    pub(crate) app_id: String,
    pub(crate) activated: bool,
    pub(crate) maximized: bool,
    pub(crate) minimized: bool,
    pub(crate) fullscreen: bool,
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for LayerState {
    fn event(
        state: &mut Self,
        _manager: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                state
                    .toplevel_controls
                    .insert(toplevel.id(), ToplevelControl::default());
                state
                    .toplevel_control_handles
                    .insert(toplevel.id(), toplevel);
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                state.toplevel_controls.clear();
                state.toplevel_control_handles.clear();
                state.toplevels_changed = true;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(LayerState, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE
            => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for LayerState {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        let key = handle.id();
        let entry = state.toplevel_controls.entry(key.clone()).or_default();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                entry.pending.title = title;
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                entry.pending.app_id = app_id;
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: bits } => {
                // A list of u32 states rather than a bitfield, four bytes each.
                // Absent means false, so the whole set is rebuilt here rather
                // than toggled — a window that stops being maximized says so by
                // not mentioning it.
                entry.pending.activated = false;
                entry.pending.maximized = false;
                entry.pending.minimized = false;
                entry.pending.fullscreen = false;
                for value in bits
                    .chunks_exact(4)
                    .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                {
                    match value {
                        value if value == State::Activated as u32 => {
                            entry.pending.activated = true;
                        }
                        value if value == State::Maximized as u32 => {
                            entry.pending.maximized = true;
                        }
                        value if value == State::Minimized as u32 => {
                            entry.pending.minimized = true;
                        }
                        value if value == State::Fullscreen as u32 => {
                            entry.pending.fullscreen = true;
                        }
                        _ => {}
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                let pending = entry.pending.clone();
                entry.title = pending.title;
                entry.app_id = pending.app_id;
                entry.activated = pending.activated;
                entry.maximized = pending.maximized;
                entry.minimized = pending.minimized;
                entry.fullscreen = pending.fullscreen;
                state.toplevels_changed = true;
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevel_controls.remove(&key);
                state.toplevel_control_handles.remove(&key);
                state.toplevels_changed = true;
                handle.destroy();
            }
            _ => {}
        }
    }
}
