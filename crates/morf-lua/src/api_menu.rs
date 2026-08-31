use luna::{Callback, CallbackReturn, Context, Table, UserData, UserRef, Value as LuaValue};
use morf_desktop::{DesktopEntries, desktop_paths};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use morf_menu::Menu;

use crate::{
    lua_values::*, reactive_execute::*, scene_bindings::*, state::*, table_menu::*, types::*,
};

pub(crate) fn install_menu_desktop_api<'gc>(ctx: Context<'gc>, morf: Table<'gc>, limits: Limits) {
    let menu_entries = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let menu: UserRef<MenuToken> = stack.consume(ctx)?;
        stack.replace(ctx, menu_entries_to_lua(ctx, menu.menu.borrow().entries()));
        Ok(CallbackReturn::Return)
    });
    let menu_children = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (menu, parent): (UserRef<MenuToken>, LuaValue) = stack.consume(ctx)?;
        let parent = match parent {
            LuaValue::Nil => None,
            LuaValue::String(value) => Some(value.display_lossy().to_string()),
            _ => return Err(HostError("menu parent must be a string or nil".into()).into()),
        };
        let menu = menu.menu.borrow();
        let children = menu
            .children(parent.as_deref())
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, menu_entries_to_lua(ctx, children));
        Ok(CallbackReturn::Return)
    });
    let menu_entry = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (menu, id): (UserRef<MenuToken>, String) = stack.consume(ctx)?;
        match menu.menu.borrow().entry(&id) {
            Some(entry) => stack.replace(ctx, menu_entry_to_lua(ctx, entry)),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let menu_activate = Callback::from_fn(&ctx, {
        move |ctx, _, mut stack| {
            let (menu, id): (UserRef<MenuToken>, String) = stack.consume(ctx)?;
            let activation = menu
                .menu
                .borrow_mut()
                .activate(&id)
                .map_err(|error| HostError(error.to_string()))?;
            let callback = menu.callbacks.get(&id).cloned();
            if let Some(callback) = callback {
                execute_handler_args(ctx, &callback, &[], limits).map_err(HostError)?;
            }
            let value = Table::new(&ctx);
            value.set_field(ctx, "id", activation.id.as_str());
            value.set_field(ctx, "check_state", check_state_name(activation.check_state));
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        }
    });
    let menu_set_enabled = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (menu, id, enabled): (UserRef<MenuToken>, String, bool) = stack.consume(ctx)?;
        menu.menu
            .borrow_mut()
            .set_enabled(&id, enabled)
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let menu_set_visible = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (menu, id, visible): (UserRef<MenuToken>, String, bool) = stack.consume(ctx)?;
        menu.menu
            .borrow_mut()
            .set_visible(&id, visible)
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let menu_set_checked = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (menu, id, checked): (UserRef<MenuToken>, String, LuaValue) = stack.consume(ctx)?;
        let checked = parse_check_state(checked).map_err(HostError)?;
        menu.menu
            .borrow_mut()
            .set_check_state(&id, checked)
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let menu_methods = Table::new(&ctx);
    menu_methods.set_field(ctx, "entries", menu_entries);
    menu_methods.set_field(ctx, "children", menu_children);
    menu_methods.set_field(ctx, "entry", menu_entry);
    menu_methods.set_field(ctx, "activate", menu_activate);
    menu_methods.set_field(ctx, "set_enabled", menu_set_enabled);
    menu_methods.set_field(ctx, "set_visible", menu_set_visible);
    menu_methods.set_field(ctx, "set_checked", menu_set_checked);
    let menu_metatable = Table::new(&ctx);
    menu_metatable.set_field(ctx, "__index", menu_methods);
    let menu_metatable = ctx.stash(menu_metatable);
    let menu = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: Table = stack.consume(ctx)?;
        let entries = match options.get_value(ctx, "entries") {
            LuaValue::Nil => options,
            LuaValue::Table(entries) => entries,
            _ => return Err(HostError("menu entries must be a table".into()).into()),
        };
        let mut callbacks = HashMap::new();
        let entries = parse_menu_entries(ctx, entries, 0, &mut callbacks).map_err(HostError)?;
        let menu = Menu::new(entries).map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            MenuToken {
                menu: RefCell::new(menu),
                callbacks,
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&menu_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "menu", menu);

    let desktop_applications = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let entries: UserRef<DesktopEntriesToken> = stack.consume(ctx)?;
        let values = Table::new(&ctx);
        for (index, entry) in entries.entries.borrow().applications().iter().enumerate() {
            values.set(ctx, index as i64 + 1, desktop_entry_table(ctx, entry))?;
        }
        stack.replace(ctx, values);
        Ok(CallbackReturn::Return)
    });
    let desktop_by_id = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (entries, id): (UserRef<DesktopEntriesToken>, String) = stack.consume(ctx)?;
        match entries.entries.borrow().by_id(&id) {
            Some(entry) => stack.replace(ctx, desktop_entry_table(ctx, entry)),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let desktop_lookup = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (entries, query): (UserRef<DesktopEntriesToken>, String) = stack.consume(ctx)?;
        match entries.entries.borrow().heuristic_lookup(&query) {
            Some(entry) => stack.replace(ctx, desktop_entry_table(ctx, entry)),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let desktop_launch = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (entries, id): (UserRef<DesktopEntriesToken>, String) = stack.consume(ctx)?;
        let entries_ref = entries.entries.borrow();
        let entry = entries_ref
            .by_id(&id)
            .ok_or_else(|| HostError(format!("desktop entry `{id}` was not found")))?;
        entry
            .launch()
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let desktop_launch_action = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (entries, id, action): (UserRef<DesktopEntriesToken>, String, String) =
            stack.consume(ctx)?;
        let entries_ref = entries.entries.borrow();
        let entry = entries_ref
            .by_id(&id)
            .ok_or_else(|| HostError(format!("desktop entry `{id}` was not found")))?;
        let action = entry
            .actions
            .iter()
            .find(|candidate| candidate.id == action)
            .ok_or_else(|| HostError(format!("desktop action `{action}` was not found")))?;
        action
            .launch(&entry.working_directory)
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let desktop_methods = Table::new(&ctx);
    desktop_methods.set_field(ctx, "applications", desktop_applications);
    desktop_methods.set_field(ctx, "by_id", desktop_by_id);
    desktop_methods.set_field(ctx, "heuristic_lookup", desktop_lookup);
    desktop_methods.set_field(ctx, "launch", desktop_launch);
    desktop_methods.set_field(ctx, "launch_action", desktop_launch_action);
    let desktop_refresh = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let entries: UserRef<DesktopEntriesToken> = stack.consume(ctx)?;
        let next = DesktopEntries::scan_paths(entries.paths.clone())
            .map_err(|error| HostError(error.to_string()))?;
        let changed = *entries.entries.borrow() != next;
        if changed {
            *entries.entries.borrow_mut() = next;
        }
        stack.replace(ctx, changed);
        Ok(CallbackReturn::Return)
    });
    desktop_methods.set_field(ctx, "refresh", desktop_refresh);
    let desktop_metatable = Table::new(&ctx);
    desktop_metatable.set_field(ctx, "__index", desktop_methods);
    let desktop_metatable = ctx.stash(desktop_metatable);
    let desktop_entries = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let paths: LuaValue = stack.consume(ctx)?;
        let paths = match paths {
            LuaValue::Nil => desktop_paths(),
            LuaValue::Table(paths) => table_string_array(ctx, paths, 256)
                .map_err(HostError)?
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>(),
            _ => {
                return Err(HostError("desktop_entries paths must be a table".into()).into());
            }
        };
        let entries = DesktopEntries::scan_paths(paths.clone())
            .map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            DesktopEntriesToken {
                entries: RefCell::new(entries),
                paths,
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&desktop_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "desktop_entries", desktop_entries);
}
