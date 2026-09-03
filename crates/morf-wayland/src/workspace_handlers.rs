//! Workspaces, without knowing whose they are.
//!
//! Every compositor has these and, until this protocol, every compositor had
//! its own way of saying so: Hyprland over its own socket, sway over i3's,
//! each with a different vocabulary. A shell that wanted a workspace indicator
//! had to grow a client per compositor, and morf's own examples reached for
//! `/dispatch workspace N` because there was nothing neutral to reach for.
//!
//! `ext-workspace-v1` is the neutral answer, and binding it is how workspaces
//! arrive here without a line of per-compositor code.
//!
//! The shape is two levels: groups, which are roughly outputs, and workspaces,
//! which belong to groups. Both arrive piecemeal and neither is worth reading
//! until the manager says `done` — the same contract the window list beside
//! this one has, and for the same reason.

use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};

use crate::{state_types::LayerState, types::WorkspaceInfo};

/// `state` is a bitfield, and these are its bits.
const STATE_ACTIVE: u32 = 1;
const STATE_URGENT: u32 = 2;
const STATE_HIDDEN: u32 = 4;

/// `capabilities` is another, and this is the only bit a configuration acts on.
const CAPABILITY_ACTIVATE: u32 = 1;

impl Dispatch<ExtWorkspaceManagerV1, ()> for LayerState {
    /// Groups and workspaces appearing, and the point at which they are true.
    ///
    /// Nothing is published before `done`. A workspace announces its id, name,
    /// coordinates and state on four separate events, and a configuration that
    /// read it in between would see a workspace with no name.
    fn event(
        state: &mut Self,
        _manager: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        match event {
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                let key = workspace.id();
                // The key a configuration acts on, decided here rather than
                // waiting for the compositor to offer one. `id` is optional in
                // the protocol -- Hyprland sends none at all -- so keying
                // activation on it would mean workspaces that cannot be
                // switched to on the compositor most likely to be running.
                // This is unique and lives exactly as long as the workspace.
                let info = WorkspaceInfo {
                    key: format!("{}", key.protocol_id()),
                    ..WorkspaceInfo::default()
                };
                state.workspaces.insert(key.clone(), info);
                state.workspace_handles.insert(key, workspace);
            }
            ext_workspace_manager_v1::Event::Done => state.workspaces_changed = true,
            ext_workspace_manager_v1::Event::Finished => {
                // The compositor has stopped talking about workspaces. The
                // handles are dead, so the list has to go with them rather than
                // stand as a snapshot that will never be corrected.
                state.workspaces.clear();
                state.workspace_handles.clear();
                state.workspaces_changed = true;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(LayerState, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for LayerState {
    /// Which output a group is on, and which workspaces belong to it.
    ///
    /// A group is how the protocol says "these workspaces live on that screen",
    /// which is the only reason a shell cares about groups at all: it is what
    /// lets a per-output bar show its own workspaces rather than all of them.
    fn event(
        state: &mut Self,
        group: &ExtWorkspaceGroupHandleV1,
        event: ext_workspace_group_handle_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        match event {
            ext_workspace_group_handle_v1::Event::OutputEnter { output } => {
                let name = state
                    .outputs
                    .info(&output)
                    .and_then(|info| info.name)
                    .unwrap_or_default();
                state.workspace_group_outputs.insert(group.id(), name);
                relabel_group(state, group);
            }
            ext_workspace_group_handle_v1::Event::OutputLeave { .. } => {
                state.workspace_group_outputs.remove(&group.id());
                relabel_group(state, group);
            }
            ext_workspace_group_handle_v1::Event::WorkspaceEnter { workspace } => {
                state
                    .workspace_groups
                    .insert(workspace.id(), group.id().clone());
                relabel_group(state, group);
            }
            ext_workspace_group_handle_v1::Event::WorkspaceLeave { workspace } => {
                state.workspace_groups.remove(&workspace.id());
                if let Some(entry) = state.workspaces.get_mut(&workspace.id()) {
                    entry.output.clear();
                }
            }
            ext_workspace_group_handle_v1::Event::Removed => {
                state.workspace_group_outputs.remove(&group.id());
                group.destroy();
            }
            _ => {}
        }
    }
}

/// Re-stamps every workspace in a group with the group's output.
///
/// The two facts arrive in either order and on different events — a workspace
/// can join a group before the group has an output, or after — so rather than
/// guess which came first, whichever arrives last recomputes from both.
fn relabel_group(state: &mut LayerState, group: &ExtWorkspaceGroupHandleV1) {
    let output = state
        .workspace_group_outputs
        .get(&group.id())
        .cloned()
        .unwrap_or_default();
    let members = state
        .workspace_groups
        .iter()
        .filter(|(_, owner)| *owner == &group.id())
        .map(|(workspace, _)| workspace.clone())
        .collect::<Vec<_>>();
    for workspace in members {
        if let Some(entry) = state.workspaces.get_mut(&workspace) {
            entry.output = output.clone();
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for LayerState {
    /// One workspace describing itself.
    fn event(
        state: &mut Self,
        workspace: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        let key = workspace.id();
        match event {
            ext_workspace_handle_v1::Event::Id { id } => {
                // Optional, and not for showing: the protocol says these are
                // sent only for workspaces the compositor expects to survive a
                // session, so a configuration can remember a preference against
                // one. Never the key -- see `key` above.
                state.workspaces.entry(key).or_default().id = id;
            }
            ext_workspace_handle_v1::Event::Name { name } => {
                state.workspaces.entry(key).or_default().name = name;
            }
            ext_workspace_handle_v1::Event::Coordinates { coordinates } => {
                // Four bytes per coordinate, native-endian, however many
                // dimensions this compositor counts in. Kept as numbers because
                // what they mean is the compositor's business; what a shell
                // does with them is sort by them.
                state.workspaces.entry(key).or_default().coordinates = coordinates
                    .chunks_exact(4)
                    .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();
            }
            ext_workspace_handle_v1::Event::State { state: bits } => {
                // A bitfield arriving as a WEnum: `into_result` fails for bit
                // combinations the generated enum has no name for, which is
                // most of them, so the raw value is what to read.
                let bits: u32 = bits.into();
                let entry = state.workspaces.entry(key).or_default();
                entry.active = bits & STATE_ACTIVE != 0;
                entry.urgent = bits & STATE_URGENT != 0;
                entry.hidden = bits & STATE_HIDDEN != 0;
            }
            ext_workspace_handle_v1::Event::Capabilities { capabilities } => {
                let capabilities: u32 = capabilities.into();
                state.workspaces.entry(key).or_default().activatable =
                    capabilities & CAPABILITY_ACTIVATE != 0;
            }
            ext_workspace_handle_v1::Event::Removed => {
                state.workspaces.remove(&key);
                state.workspace_handles.remove(&key);
                state.workspace_groups.remove(&key);
                state.workspaces_changed = true;
                workspace.destroy();
            }
            _ => {}
        }
    }
}
