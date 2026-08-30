fn lua_ipc_value(value: &WireValue) -> IpcValue {
    match value {
        WireValue::Nil => IpcValue::Nil,
        WireValue::Boolean(value) => IpcValue::Boolean(*value),
        WireValue::Integer(value) => IpcValue::Integer(*value),
        WireValue::Number(value) => IpcValue::Number(*value),
        WireValue::String(value) => IpcValue::String(value.clone()),
    }
}

fn wire_ipc_value(value: &IpcValue) -> WireValue {
    match value {
        IpcValue::Nil => WireValue::Nil,
        IpcValue::Boolean(value) => WireValue::Boolean(*value),
        IpcValue::Integer(value) => WireValue::Integer(*value),
        IpcValue::Number(value) => WireValue::Number(*value),
        IpcValue::String(value) => WireValue::String(value.clone()),
    }
}

fn apply_output_power_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for on in runtime.take_output_power_requests() {
        client.set_output_power(if on {
            OutputPowerMode::On
        } else {
            OutputPowerMode::Off
        });
    }
}

fn apply_clipboard_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    if !client.can_set_clipboard() {
        return;
    }
    for text in runtime.take_clipboard_requests() {
        client.set_clipboard(text);
    }
}

fn apply_screencopy_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for request in runtime.take_screencopy_requests() {
        if !client.capture_output(request.id, request.include_cursor) {
            runtime.dispatch_screencopy(request.id, Err("screencopy is unavailable".to_owned()));
        }
    }
}

fn dispatch_screencopy(
    runtime: &mut Runtime,
    request_id: u64,
    result: Result<mold_wayland::ScreencopyFrame, String>,
) -> bool {
    runtime.dispatch_screencopy(
        request_id,
        result.map(|frame| LuaScreencopy {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            format: match frame.format {
                ScreencopyFormat::Argb8888 => "argb8888",
                ScreencopyFormat::Xrgb8888 => "xrgb8888",
            }
            .to_owned(),
            y_invert: frame.y_invert,
            pixels: frame.pixels,
        }),
    )
}

fn apply_virtual_keyboard_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for request in runtime.take_virtual_keyboard_requests() {
        match request {
            VirtualKeyboardRequest::Key { keycode, pressed } => {
                client.send_virtual_key(keycode, pressed);
            }
            VirtualKeyboardRequest::Modifiers {
                depressed,
                latched,
                locked,
                group,
            } => {
                client.send_virtual_modifiers(depressed, latched, locked, group);
            }
        }
    }
}

fn apply_input_method_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    if runtime.take_input_method_enable_request() {
        client.enable_input_method();
    }
    for request in runtime.take_input_method_requests() {
        match request {
            InputMethodRequest::Commit(text) => {
                client.input_method_commit(&text);
            }
            InputMethodRequest::Preedit { text, begin, end } => {
                client.input_method_preedit(&text, begin, end);
            }
            InputMethodRequest::Delete { before, after } => {
                client.input_method_delete(before, after);
            }
        }
    }
}

fn apply_text_input_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    if runtime.take_text_input_enable_request() {
        client.enable_text_input();
    }
    for request in runtime.take_text_input_requests() {
        match request {
            TextInputRequest::Disable => {
                client.disable_text_input();
            }
            TextInputRequest::Surrounding {
                text,
                cursor,
                anchor,
            } => {
                client.set_text_input_surrounding(&text, cursor, anchor);
            }
            TextInputRequest::ContentType { hints, purpose } => {
                client.set_text_input_content_type(hints, purpose);
            }
            TextInputRequest::CursorRect {
                x,
                y,
                width,
                height,
            } => {
                client.set_text_input_cursor_rect(InputRect {
                    x,
                    y,
                    width,
                    height,
                });
            }
        }
    }
}

fn stop_workers(workers: BTreeMap<String, Worker>) {
    for worker in workers.values() {
        worker.stop.store(true, Ordering::Release);
    }
    for (_, worker) in workers {
        let _ = worker.join.join();
    }
}

