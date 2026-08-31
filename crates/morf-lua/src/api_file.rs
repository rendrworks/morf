use luna::{Callback, CallbackReturn, Context, Table, UserData, UserRef, Value as LuaValue};
use morf_io::{FileDocument, FileEvent, FileView};
use std::cell::RefCell;

use crate::{lua_values::*, scene_bindings::*, state::*};

pub(crate) fn install_file_api<'gc>(ctx: Context<'gc>, morf: Table<'gc>) {
    let file_read = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileToken> = stack.consume(ctx)?;
        let bytes = file
            .file
            .read_bounded(1024 * 1024)
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, String::from_utf8_lossy(&bytes).as_ref());
        Ok(CallbackReturn::Return)
    });
    let file_write = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (file, bytes): (UserRef<FileToken>, String) = stack.consume(ctx)?;
        if bytes.len() > 1024 * 1024 {
            return Err(HostError("file write exceeds 1 MiB".to_owned()).into());
        }
        file.file
            .write(bytes.as_bytes())
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let watcher_next = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (watcher, timeout_ms): (UserRef<FileWatcherToken>, i64) = stack.consume(ctx)?;
        let timeout = bounded_timeout(timeout_ms).map_err(HostError)?;
        let event = watcher.watcher.next_event(timeout);
        match event {
            Some(FileEvent::Changed) => stack.replace(ctx, "changed"),
            Some(FileEvent::Moved) => stack.replace(ctx, "moved"),
            Some(FileEvent::Deleted) => stack.replace(ctx, "deleted"),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let watcher_methods = Table::new(&ctx);
    watcher_methods.set_field(ctx, "next", watcher_next);
    let watcher_metatable = Table::new(&ctx);
    watcher_metatable.set_field(ctx, "__index", watcher_methods);
    let watcher_metatable = ctx.stash(watcher_metatable);
    let file_watch = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let file: UserRef<FileToken> = stack.consume(ctx)?;
        let watcher = file
            .file
            .watch()
            .map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(&ctx, FileWatcherToken { watcher });
        userdata.set_metatable(ctx, Some(ctx.fetch(&watcher_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    let file_methods = Table::new(&ctx);
    file_methods.set_field(ctx, "read", file_read);
    file_methods.set_field(ctx, "write", file_write);
    file_methods.set_field(ctx, "watch", file_watch);
    let file_metatable = Table::new(&ctx);
    file_metatable.set_field(ctx, "__index", file_methods);
    let file_metatable = ctx.stash(file_metatable);
    let file_view = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let path: String = stack.consume(ctx)?;
        let userdata = UserData::new_static(
            &ctx,
            FileToken {
                file: FileView::new(path),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&file_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "file", file_view);

    let document_path = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        stack.replace(ctx, file.file.borrow().path().to_string_lossy().as_ref());
        Ok(CallbackReturn::Return)
    });
    let document_set_path = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (file, path, preload): (UserRef<FileDocumentToken>, String, LuaValue) =
            stack.consume(ctx)?;
        let preload = match preload {
            LuaValue::Nil => file.file.borrow().preload(),
            LuaValue::Boolean(value) => value,
            _ => return Err(HostError("preload must be boolean".into()).into()),
        };
        let mut file = file.file.borrow_mut();
        file.set_preload(preload);
        file.set_path(&path)
            .map_err(|error| HostError(error.to_string()))?;
        let loaded = path.is_empty() || !preload || file.reload();
        stack.replace(ctx, loaded);
        Ok(CallbackReturn::Return)
    });
    let document_preload = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        stack.replace(ctx, file.file.borrow().preload());
        Ok(CallbackReturn::Return)
    });
    let document_set_preload = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (file, preload): (UserRef<FileDocumentToken>, bool) = stack.consume(ctx)?;
        let mut file = file.file.borrow_mut();
        file.set_preload(preload);
        let loaded =
            !preload || file.loaded() || file.path().as_os_str().is_empty() || file.reload();
        stack.replace(ctx, loaded);
        Ok(CallbackReturn::Return)
    });
    let document_reload = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        let loaded = file.file.borrow_mut().reload();
        stack.replace(ctx, loaded);
        Ok(CallbackReturn::Return)
    });
    let document_loaded = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        stack.replace(ctx, file.file.borrow().loaded());
        Ok(CallbackReturn::Return)
    });
    let document_exists = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        stack.replace(ctx, file.file.borrow().exists());
        Ok(CallbackReturn::Return)
    });
    let document_error = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        match file.file.borrow().error() {
            Some(error) => stack.replace(ctx, error.as_str()),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let document_text = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        match file.file.borrow().text() {
            Some(text) => stack.replace(ctx, text),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let document_data = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        match file.file.borrow().data() {
            Some(data) => stack.replace(ctx, ctx.intern(data)),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let document_set_data = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (file, data): (UserRef<FileDocumentToken>, String) = stack.consume(ctx)?;
        let saved = file.file.borrow_mut().set_data(data.as_bytes());
        stack.replace(ctx, saved);
        Ok(CallbackReturn::Return)
    });
    let document_atomic = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        stack.replace(ctx, file.file.borrow().atomic_writes());
        Ok(CallbackReturn::Return)
    });
    let document_set_atomic = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (file, atomic): (UserRef<FileDocumentToken>, bool) = stack.consume(ctx)?;
        file.file.borrow_mut().set_atomic_writes(atomic);
        Ok(CallbackReturn::Return)
    });
    let document_watching = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
        stack.replace(ctx, file.file.borrow().watch_changes());
        Ok(CallbackReturn::Return)
    });
    let document_set_watching = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (file, enabled): (UserRef<FileDocumentToken>, bool) = stack.consume(ctx)?;
        file.file
            .borrow_mut()
            .set_watch_changes(enabled)
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let document_next_change = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (file, timeout_ms): (UserRef<FileDocumentToken>, i64) = stack.consume(ctx)?;
        let timeout = bounded_timeout(timeout_ms).map_err(HostError)?;
        match file.file.borrow().next_change(timeout) {
            Some(FileEvent::Changed) => stack.replace(ctx, "changed"),
            Some(FileEvent::Moved) => stack.replace(ctx, "moved"),
            Some(FileEvent::Deleted) => stack.replace(ctx, "deleted"),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let document_methods = Table::new(&ctx);
    document_methods.set_field(ctx, "path", document_path);
    document_methods.set_field(ctx, "set_path", document_set_path);
    document_methods.set_field(ctx, "preload", document_preload);
    document_methods.set_field(ctx, "set_preload", document_set_preload);
    document_methods.set_field(ctx, "reload", document_reload);
    document_methods.set_field(ctx, "loaded", document_loaded);
    document_methods.set_field(ctx, "exists", document_exists);
    document_methods.set_field(ctx, "error", document_error);
    document_methods.set_field(ctx, "text", document_text);
    document_methods.set_field(ctx, "data", document_data);
    document_methods.set_field(ctx, "set_text", document_set_data);
    document_methods.set_field(ctx, "set_data", document_set_data);
    document_methods.set_field(ctx, "atomic_writes", document_atomic);
    document_methods.set_field(ctx, "set_atomic_writes", document_set_atomic);
    document_methods.set_field(ctx, "watch_changes", document_watching);
    document_methods.set_field(ctx, "set_watch_changes", document_set_watching);
    document_methods.set_field(ctx, "next_change", document_next_change);
    let document_metatable = Table::new(&ctx);
    document_metatable.set_field(ctx, "__index", document_methods);
    let document_metatable = ctx.stash(document_metatable);
    let file_document = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: Table = stack.consume(ctx)?;
        let path = match options.get_value(ctx, "path") {
            LuaValue::String(path) => path.display_lossy().to_string(),
            _ => return Err(HostError("file_view path must be a string".into()).into()),
        };
        let preload = match options.get_value(ctx, "preload") {
            LuaValue::Nil => true,
            LuaValue::Boolean(value) => value,
            _ => return Err(HostError("file_view preload must be boolean".into()).into()),
        };
        let watch_changes = match options.get_value(ctx, "watch_changes") {
            LuaValue::Nil => false,
            LuaValue::Boolean(value) => value,
            _ => {
                return Err(HostError("file_view watch_changes must be boolean".into()).into());
            }
        };
        let atomic_writes = match options.get_value(ctx, "atomic_writes") {
            LuaValue::Nil => true,
            LuaValue::Boolean(value) => value,
            _ => {
                return Err(HostError("file_view atomic_writes must be boolean".into()).into());
            }
        };
        let maximum = match options.get_value(ctx, "maximum_bytes") {
            LuaValue::Nil => 1024 * 1024,
            LuaValue::Integer(value) => usize::try_from(value)
                .ok()
                .filter(|value| (1..=16 * 1024 * 1024).contains(value))
                .ok_or_else(|| HostError("file_view maximum_bytes must be 1..16777216".into()))?,
            _ => {
                return Err(HostError("file_view maximum_bytes must be an integer".into()).into());
            }
        };
        let mut file = FileDocument::new(path, maximum);
        file.set_preload(preload);
        file.set_atomic_writes(atomic_writes);
        if preload {
            file.reload();
        }
        if watch_changes {
            file.set_watch_changes(true)
                .map_err(|error| HostError(error.to_string()))?;
        }
        let userdata = UserData::new_static(
            &ctx,
            FileDocumentToken {
                file: RefCell::new(file),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&document_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "file_view", file_document);
}
