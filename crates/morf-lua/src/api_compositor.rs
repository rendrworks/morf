//! Acting on the compositor: its workspaces, and other people's windows.
//!
//! Split from `api_host` at the line gate, and it is a fair seam. Everything
//! left there describes *this* shell — its screens, its idle timeouts, its
//! clipboard. These two describe the session around it, and both are lists the
//! configuration reads and requests it makes back.

use luna::{Callback, CallbackReturn, Closure, Context, Table};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    scene_bindings::HostError,
    state::*,
    types::{ToplevelRequest, WorkspaceRequest},
};

/// Queues one workspace request, refusing a configuration that queues
/// hundreds in a frame: that is a loop, not a shell.
fn push_workspace_request(
    state: &Rc<RefCell<ReactiveState>>,
    request: WorkspaceRequest,
) -> Result<(), HostError> {
    let mut state = state.borrow_mut();
    if state.workspace_requests.len() >= 64 {
        return Err(HostError("workspace request limit reached".into()));
    }
    state.workspace_requests.push(request);
    Ok(())
}

/// Installs `morf.workspaces`, `morf.workspace` and `morf.toplevel`.
pub(crate) fn install_compositor_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let workspaces = Table::new(&ctx);
    morf.set_field(ctx, "workspaces", workspaces);
    // Three verbs on a workspace, and each is honest about being refusable:
    // the compositor says per workspace which it will honour, and that is what
    // `activatable`, `removable` and `assignable` on the entry carry.
    let workspace = Table::new(&ctx);
    let activate_state = Rc::clone(&state);
    let workspace_activate = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let key: String = stack.consume(ctx)?;
        push_workspace_request(&activate_state, WorkspaceRequest::Activate(key))?;
        Ok(CallbackReturn::Return)
    });
    let remove_state = Rc::clone(&state);
    let workspace_remove = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let key: String = stack.consume(ctx)?;
        push_workspace_request(&remove_state, WorkspaceRequest::Remove(key))?;
        Ok(CallbackReturn::Return)
    });
    let assign_state = Rc::clone(&state);
    let workspace_assign = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (key, output): (String, String) = stack.consume(ctx)?;
        push_workspace_request(&assign_state, WorkspaceRequest::Assign { key, output })?;
        Ok(CallbackReturn::Return)
    });
    workspace.set_field(ctx, "activate", workspace_activate);
    workspace.set_field(ctx, "remove", workspace_remove);
    workspace.set_field(ctx, "assign", workspace_assign);
    morf.set_field(ctx, "workspace", workspace);

    // Acting on somebody else's window. Named for the protocol rather than
    // called `morf.window`, which is this shell's own surface constructor and a
    // different thing entirely.
    let toplevel = Table::new(&ctx);
    for (name, takes_value) in [
        ("activate", false),
        ("close", false),
        ("set_maximized", true),
        ("set_minimized", true),
        ("set_fullscreen", true),
    ] {
        let request_state = Rc::clone(&state);
        let action = name.to_owned();
        let entry = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (identifier, value): (String, Option<bool>) = stack.consume(ctx)?;
            let mut state = request_state.borrow_mut();
            if state.toplevel_requests.len() >= 64 {
                return Err(HostError("window request limit reached".into()).into());
            }
            state.toplevel_requests.push(ToplevelRequest {
                identifier,
                action: action.clone(),
                // The setters default to turning the state on, so
                // `set_maximized(id)` reads the way it looks.
                value: if takes_value {
                    value.unwrap_or(true)
                } else {
                    true
                },
                rect: None,
            });
            Ok(CallbackReturn::Return)
        });
        toplevel.set_field(ctx, name, entry);
    }
    // Where a window's entry is, so a compositor that animates minimize has
    // somewhere to send it. Coordinates on the shell's own surface.
    let target_state = Rc::clone(&state);
    let set_minimize_target = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (identifier, x, y, width, height): (String, i64, i64, i64, i64) = stack.consume(ctx)?;
        let narrow = |value: i64| {
            i32::try_from(value)
                .map_err(|_| HostError(format!("`{value}` does not fit a surface coordinate")))
        };
        let mut state = target_state.borrow_mut();
        if state.toplevel_requests.len() >= 64 {
            return Err(HostError("window request limit reached".into()).into());
        }
        state.toplevel_requests.push(ToplevelRequest {
            identifier,
            action: "set_minimize_target".to_owned(),
            value: true,
            rect: Some((narrow(x)?, narrow(y)?, narrow(width)?, narrow(height)?)),
        });
        Ok(CallbackReturn::Return)
    });
    toplevel.set_field(ctx, "set_minimize_target", set_minimize_target);
    morf.set_field(ctx, "toplevel", toplevel);

    // The compositor's own key bindings, held off the shell while it has
    // focus. `active` is the compositor's answer, which is not always yes.
    let shortcuts_state = Rc::clone(&state);
    let shortcuts_inhibit = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let inhibited: bool = stack.consume(ctx)?;
        let mut state = shortcuts_state.borrow_mut();
        state.shortcuts_inhibited = inhibited;
        state.shortcuts_inhibit_changed = true;
        Ok(CallbackReturn::Return)
    });
    // The compositor's answer, delivered rather than polled: a binding that
    // read a plain function would never re-run when the answer changed.
    let subscribe_state = Rc::clone(&state);
    let shortcuts_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let callback: Closure = stack.consume(ctx)?;
        let mut state = subscribe_state.borrow_mut();
        if state.shortcuts_callbacks.len() >= 64 {
            return Err(HostError("shortcuts callback limit reached".into()).into());
        }
        state.shortcuts_callbacks.push(ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    let shortcuts = Table::new(&ctx);
    shortcuts.set_field(ctx, "inhibit", shortcuts_inhibit);
    shortcuts.set_field(ctx, "subscribe", shortcuts_subscribe);
    morf.set_field(ctx, "shortcuts", shortcuts);
}
