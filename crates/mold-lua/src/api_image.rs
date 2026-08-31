use luna::{Callback, CallbackReturn, Context, Table, UserData, UserRef, Value as LuaValue};
use mold_image::{IconResolver, quantize_colors};
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};

use crate::{process_helpers::*, scene_bindings::*, state::*, table_menu::*, types::*};

pub(crate) fn install_image_api<'gc>(ctx: Context<'gc>, mold: Table<'gc>) {
    let quantizer_colors = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let quantizer: UserRef<ColorQuantizerToken> = stack.consume(ctx)?;
        stack.replace(
            ctx,
            quantizer_colors_to_lua(ctx, &quantizer.state.borrow().colors),
        );
        Ok(CallbackReturn::Return)
    });
    let quantizer_source = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let quantizer: UserRef<ColorQuantizerToken> = stack.consume(ctx)?;
        stack.replace(
            ctx,
            quantizer.state.borrow().source.to_string_lossy().as_ref(),
        );
        Ok(CallbackReturn::Return)
    });
    let quantizer_set_source = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (quantizer, source): (UserRef<ColorQuantizerToken>, String) = stack.consume(ctx)?;
        update_color_quantizer(&quantizer, |state| state.source = PathBuf::from(source))
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let quantizer_depth = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let quantizer: UserRef<ColorQuantizerToken> = stack.consume(ctx)?;
        stack.replace(ctx, i64::from(quantizer.state.borrow().depth));
        Ok(CallbackReturn::Return)
    });
    let quantizer_set_depth = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (quantizer, depth): (UserRef<ColorQuantizerToken>, i64) = stack.consume(ctx)?;
        let depth = u8::try_from(depth)
            .ok()
            .filter(|value| *value <= 8)
            .ok_or_else(|| HostError("color_quantizer depth must be 0..8".into()))?;
        update_color_quantizer(&quantizer, |state| state.depth = depth).map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let quantizer_rescale_size = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let quantizer: UserRef<ColorQuantizerToken> = stack.consume(ctx)?;
        stack.replace(ctx, i64::from(quantizer.state.borrow().rescale_size));
        Ok(CallbackReturn::Return)
    });
    let quantizer_set_rescale_size = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (quantizer, size): (UserRef<ColorQuantizerToken>, i64) = stack.consume(ctx)?;
        let size = u32::try_from(size)
            .ok()
            .filter(|value| *value <= 512)
            .ok_or_else(|| HostError("color_quantizer rescale_size must be 0..512".into()))?;
        update_color_quantizer(&quantizer, |state| state.rescale_size = size).map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let quantizer_rect = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let quantizer: UserRef<ColorQuantizerToken> = stack.consume(ctx)?;
        match quantizer.state.borrow().crop {
            Some(rect) => {
                let value = Table::new(&ctx);
                value.set_field(ctx, "x", i64::from(rect.x));
                value.set_field(ctx, "y", i64::from(rect.y));
                value.set_field(ctx, "width", i64::from(rect.width));
                value.set_field(ctx, "height", i64::from(rect.height));
                stack.replace(ctx, value);
            }
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let quantizer_set_rect = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (quantizer, rect): (UserRef<ColorQuantizerToken>, LuaValue) = stack.consume(ctx)?;
        let rect = parse_quantizer_rect(ctx, rect).map_err(HostError)?;
        update_color_quantizer(&quantizer, |state| state.crop = rect).map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let quantizer_refresh = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let quantizer: UserRef<ColorQuantizerToken> = stack.consume(ctx)?;
        update_color_quantizer(&quantizer, |_| {}).map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let quantizer_methods = Table::new(&ctx);
    quantizer_methods.set_field(ctx, "colors", quantizer_colors);
    quantizer_methods.set_field(ctx, "source", quantizer_source);
    quantizer_methods.set_field(ctx, "set_source", quantizer_set_source);
    quantizer_methods.set_field(ctx, "depth", quantizer_depth);
    quantizer_methods.set_field(ctx, "set_depth", quantizer_set_depth);
    quantizer_methods.set_field(ctx, "rescale_size", quantizer_rescale_size);
    quantizer_methods.set_field(ctx, "set_rescale_size", quantizer_set_rescale_size);
    quantizer_methods.set_field(ctx, "rect", quantizer_rect);
    quantizer_methods.set_field(ctx, "set_rect", quantizer_set_rect);
    quantizer_methods.set_field(ctx, "refresh", quantizer_refresh);
    let quantizer_metatable = Table::new(&ctx);
    quantizer_metatable.set_field(ctx, "__index", quantizer_methods);
    let quantizer_metatable = ctx.stash(quantizer_metatable);
    let color_quantizer = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: Table = stack.consume(ctx)?;
        let (source, depth, crop, rescale_size) =
            parse_quantizer_options(ctx, options).map_err(HostError)?;
        let colors = quantize_colors(&source, depth, crop, rescale_size)
            .map_err(|error| HostError(error.to_string()))?;
        let value = UserData::new_static(
            &ctx,
            ColorQuantizerToken {
                state: RefCell::new(ColorQuantizerState {
                    source,
                    depth,
                    crop,
                    rescale_size,
                    colors,
                }),
            },
        );
        value.set_metatable(ctx, Some(ctx.fetch(&quantizer_metatable)));
        stack.replace(ctx, value);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "color_quantizer", color_quantizer);
    let icon_path = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (name, theme, size): (String, Option<String>, Option<i64>) = stack.consume(ctx)?;
        let (theme, size) = icon_lookup_options(&name, theme, size).map_err(HostError)?;
        match IconResolver::from_environment().find(&name, &theme, size) {
            Ok(path) => stack.replace(ctx, path.to_string_lossy().as_ref()),
            Err(_) => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "icon_path", icon_path);
    let has_icon = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (name, theme, size): (String, Option<String>, Option<i64>) = stack.consume(ctx)?;
        let (theme, size) = icon_lookup_options(&name, theme, size).map_err(HostError)?;
        stack.replace(
            ctx,
            IconResolver::from_environment()
                .find(&name, &theme, size)
                .is_ok(),
        );
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "has_icon", has_icon);
    let exec_detached = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let command: Table = stack.consume(ctx)?;
        let mut command = table_string_array(ctx, command, 64).map_err(HostError)?;
        if command.is_empty() {
            return Err(HostError("detached command cannot be empty".into()).into());
        }
        let program = command.remove(0);
        let mut child = StdCommand::new(program)
            .args(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| HostError(error.to_string()))?;
        let id = child.id();
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        stack.replace(ctx, i64::from(id));
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "exec_detached", exec_detached);
}
