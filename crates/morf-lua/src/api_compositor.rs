//! Acting on the compositor: its workspaces, and other people's windows.
//!
//! Split from `api_host` at the line gate, and it is a fair seam. Everything
//! left there describes *this* shell — its screens, its idle timeouts, its
//! clipboard. These two describe the session around it, and both are lists the
//! configuration reads and requests it makes back.

use luna::{Callback, CallbackReturn, Context, Table};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{scene_bindings::HostError, state::*, types::ToplevelRequest};

/// Installs `morf.workspaces`, `morf.workspace` and `morf.toplevel`.
pub(crate) fn install_compositor_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let workspaces = Table::new(&ctx);
    morf.set_field(ctx, "workspaces", workspaces);
    let activate_state = Rc::clone(&state);
    let workspace_activate = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let id: String = stack.consume(ctx)?;
        activate_state.borrow_mut().workspace_activation = Some(id);
        Ok(CallbackReturn::Return)
    });
    let workspace = Table::new(&ctx);
    workspace.set_field(ctx, "activate", workspace_activate);
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
            });
            Ok(CallbackReturn::Return)
        });
        toplevel.set_field(ctx, name, entry);
    }
    morf.set_field(ctx, "toplevel", toplevel);
}
