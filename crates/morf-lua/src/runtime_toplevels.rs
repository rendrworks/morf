//! `morf.windows` — every window the compositor reports.
//!
//! Kept beside the screen list rather than folded into it because they answer
//! different questions and change at different rates: outputs are a handful of
//! things that move when hardware does, and windows are dozens of things that
//! move when a person does.

use luna::{Table, Value as LuaValue};

use crate::types::*;

impl Runtime {
    /// Replaces `morf.windows` with the compositor's current window list.
    ///
    /// Updated in place, so a configuration that captured `morf.windows` keeps
    /// seeing the live list — the same contract `morf.screens` has, and for the
    /// same reason: a configuration should be able to hold the list and watch
    /// it rather than having to ask for it again.
    ///
    /// Each entry carries `identifier`, `title` and `app_id`. The identifier is
    /// the one to key on: titles change while a person reads them, and two
    /// windows of one application share an app id.
    pub fn set_windows(&mut self, windows: &[Toplevel]) {
        self.lua.enter(|ctx| {
            let Ok(morf) = ctx.get_global::<Table>("morf") else {
                return;
            };
            let LuaValue::Table(table) = morf.get_value(ctx, "windows") else {
                return;
            };
            // Cleared and refilled rather than diffed. The list is short, it is
            // rebuilt only when the compositor says something changed, and a
            // diff would have to answer what identity means for a window that
            // was renamed — which is exactly the question the identifier exists
            // to stop anybody asking.
            let previous = table.length(&ctx);
            for index in 1..=previous {
                let _ = table.set(ctx, index, LuaValue::Nil);
            }
            for (index, window) in windows.iter().enumerate() {
                let entry = Table::new(&ctx);
                entry.set_field(
                    ctx,
                    "identifier",
                    LuaValue::String(ctx.intern(window.identifier.as_bytes())),
                );
                entry.set_field(
                    ctx,
                    "title",
                    LuaValue::String(ctx.intern(window.title.as_bytes())),
                );
                entry.set_field(
                    ctx,
                    "app_id",
                    LuaValue::String(ctx.intern(window.app_id.as_bytes())),
                );
                let _ = table.set(ctx, index as i64 + 1, entry);
            }
        });
    }
}
