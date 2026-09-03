use morf_io::IpcValue as WireValue;
use morf_lua::{
    InputMethodRequest, IpcValue, Runtime, Screencopy as LuaScreencopy, TextInputRequest,
    VirtualKeyboardRequest,
};
use morf_render::{RenderEngine, WgpuBackend};
use morf_wayland::{InputRect, LayerClient, OutputPowerMode, ScreencopyFormat};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use crate::lock::*;

pub(crate) fn lua_ipc_value(value: &WireValue) -> IpcValue {
    match value {
        WireValue::Nil => IpcValue::Nil,
        WireValue::Boolean(value) => IpcValue::Boolean(*value),
        WireValue::Integer(value) => IpcValue::Integer(*value),
        WireValue::Number(value) => IpcValue::Number(*value),
        WireValue::String(value) => IpcValue::String(value.clone()),
    }
}

pub(crate) fn wire_ipc_value(value: &IpcValue) -> WireValue {
    match value {
        IpcValue::Nil => WireValue::Nil,
        IpcValue::Boolean(value) => WireValue::Boolean(*value),
        IpcValue::Integer(value) => WireValue::Integer(*value),
        IpcValue::Number(value) => WireValue::Number(*value),
        IpcValue::String(value) => WireValue::String(value.clone()),
    }
}

pub(crate) fn apply_idle_inhibit(runtime: &mut Runtime, client: &mut LayerClient) {
    if let Some(inhibited) = runtime.take_idle_inhibit_change() {
        client.set_idle_inhibited(inhibited);
    }
}

pub(crate) fn apply_shortcuts_inhibit(runtime: &mut Runtime, client: &mut LayerClient) {
    if let Some(inhibited) = runtime.take_shortcuts_inhibit_change() {
        client.set_shortcuts_inhibited(inhibited);
    }
}

pub(crate) fn apply_output_power_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for on in runtime.take_output_power_requests() {
        client.set_output_power(if on {
            OutputPowerMode::On
        } else {
            OutputPowerMode::Off
        });
    }
}

pub(crate) fn apply_clipboard_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    if !client.can_set_clipboard() {
        return;
    }
    for text in runtime.take_clipboard_requests() {
        client.set_clipboard(text);
    }
}

pub(crate) fn apply_screencopy_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for request in runtime.take_screencopy_requests() {
        // A window if one was named, an output otherwise — and for an output,
        // the newer protocol where the compositor has it.
        //
        // `wlr-screencopy` is deprecated but not dead, and it is the only path
        // on compositors that have not caught up. It is also the only one that
        // can include the cursor: the newer protocol makes a pointer a separate
        // session, so asking for one here would be asking for a different
        // capture rather than the same one with a pointer drawn on it.
        let started = match &request.window {
            Some(identifier) => client.capture_window(request.id, identifier),
            None if request.include_cursor => client.capture_output(request.id, true),
            None => {
                client.capture_output_image(request.id) || client.capture_output(request.id, false)
            }
        };
        if !started {
            let why = match &request.window {
                Some(_) if !client.supports_window_capture() => {
                    "this compositor cannot capture a single window"
                }
                Some(_) => "no window with that identifier",
                None => "screen capture is unavailable",
            };
            runtime.dispatch_screencopy(request.id, Err(why.to_owned()));
        }
    }
}

pub(crate) fn dispatch_screencopy(
    runtime: &mut Runtime,
    // Optional because not every place a capture can complete has one: the
    // configure loop runs before a renderer exists, and the lock screen keeps
    // one per output rather than one. Without a renderer the pixels still reach
    // the configuration; only the ready-made image source does not.
    renderer: Option<&mut RenderEngine<WgpuBackend>>,
    request_id: u64,
    result: Result<morf_wayland::ScreencopyFrame, String>,
) -> bool {
    // Published where `ui.Image` can find it, before the configuration is told
    // the capture arrived — so a handler can put the thumbnail straight into
    // its scene rather than being handed pixels with nowhere to go.
    //
    // Named by the request, so a thumbnail that refreshes replaces itself
    // instead of leaking an image per frame.
    if let (Some(renderer), Ok(frame)) = (renderer, &result) {
        renderer.backend_mut().publish_image(
            format!("capture/{request_id}"),
            frame.width,
            frame.height,
            capture_rgba(frame),
        );
    }
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
            source: format!("memory:capture/{request_id}"),
            pixels: frame.pixels,
        }),
    )
}

/// A capture's pixels as straight RGBA, whatever order they arrived in.
///
/// Compositors hand these over as BGRA under the names `argb8888` and
/// `xrgb8888` — little-endian, so the bytes run blue, green, red, alpha — while
/// everything downstream of here is RGBA. Getting this backwards does not fail,
/// it just makes every thumbnail look like a colour negative, which is the kind
/// of wrong that survives review.
///
/// `xrgb8888` has no alpha channel to speak of; the fourth byte is padding, and
/// leaving it as whatever the compositor put there gives a transparent
/// thumbnail.
fn capture_rgba(frame: &morf_wayland::ScreencopyFrame) -> Vec<u8> {
    let opaque = matches!(frame.format, ScreencopyFormat::Xrgb8888);
    let mut rgba = Vec::with_capacity(frame.pixels.len());
    for row in 0..frame.height as usize {
        let start = row * frame.stride as usize;
        let end = start + (frame.width as usize) * 4;
        let Some(line) = frame.pixels.get(start..end) else {
            break;
        };
        for pixel in line.chunks_exact(4) {
            rgba.extend_from_slice(&[
                pixel[2],
                pixel[1],
                pixel[0],
                if opaque { 255 } else { pixel[3] },
            ]);
        }
    }
    rgba
}

pub(crate) fn apply_virtual_keyboard_requests(runtime: &mut Runtime, client: &mut LayerClient) {
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

pub(crate) fn apply_input_method_requests(runtime: &mut Runtime, client: &mut LayerClient) {
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

pub(crate) fn apply_text_input_requests(runtime: &mut Runtime, client: &mut LayerClient) {
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

pub(crate) fn stop_workers(workers: BTreeMap<String, Worker>) {
    for worker in workers.values() {
        worker.stop.store(true, Ordering::Release);
    }
    for (_, worker) in workers {
        let _ = worker.join.join();
    }
}
