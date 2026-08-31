use luna::{Table, Value as LuaValue};

use crate::{api_host::*, types::*};

impl Runtime {
    /// Replaces `mold.screens` with the compositor's current output list.
    ///
    /// The order is part of the contract Lua configurations rely on:
    ///
    /// 1. index 1 is the output this runtime was created for — matched by name
    ///    in `screens`, so its geometry follows the compositor, and left as it
    ///    was if the compositor no longer lists it (the supervisor tears such a
    ///    runtime down anyway);
    /// 2. every other output follows in the order it was passed in, which is
    ///    the order the compositor advertised it.
    ///
    /// A runtime built without an output of its own keeps the list exactly as
    /// the compositor advertised it.
    ///
    /// The table is updated in place, so a configuration that captured
    /// `mold.screens` keeps seeing the live list. An empty list is a no-op: a
    /// runtime with no compositor behind it (`mold check`, the lock screen)
    /// keeps whatever `Runtime::for_screen` installed.
    pub fn set_screens(&mut self, screens: &[Screen]) {
        if screens.is_empty() {
            return;
        }
        self.lua.enter(|ctx| {
            let Ok(mold) = ctx.get_global::<Table>("mold") else {
                return;
            };
            let LuaValue::Table(table) = mold.get_value(ctx, "screens") else {
                return;
            };
            let own = match table.get_value(ctx, 1) {
                LuaValue::Table(entry) => Some(entry),
                _ => None,
            };
            let own_name = own.and_then(|entry| match entry.get_value(ctx, "name") {
                LuaValue::String(name) => name.to_str().ok().map(str::to_owned),
                _ => None,
            });
            // By position rather than by name, so outputs the compositor left
            // unnamed cannot collapse into one another.
            let own_index = own_name
                .as_deref()
                .and_then(|name| screens.iter().position(|screen| screen.name == name));
            let mut ordered = Vec::with_capacity(screens.len() + 1);
            match own_index {
                Some(index) => ordered.push(screen_entry(ctx, &screens[index])),
                None if own_name.is_some() => ordered.extend(own),
                None => {}
            }
            ordered.extend(
                screens
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| Some(*index) != own_index)
                    .map(|(_, screen)| screen_entry(ctx, screen)),
            );
            for (offset, entry) in ordered.iter().enumerate() {
                table
                    .set(ctx, offset as i64 + 1, *entry)
                    .expect("screen table accepts integer keys");
            }
            // Outputs come and go: whatever the previous list left past the new
            // end has to disappear rather than linger as a stale entry.
            let mut index = ordered.len() as i64 + 1;
            while !matches!(table.get_value(ctx, index), LuaValue::Nil) {
                table
                    .set(ctx, index, LuaValue::Nil)
                    .expect("screen table accepts integer keys");
                index += 1;
            }
        });
    }
}
