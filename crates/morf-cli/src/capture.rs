//! Screen captures, and where their pixels go.
//!
//! Two paths end here. Through shared memory the compositor copies the
//! frame out and this engine uploads it back, which is two trips across the
//! bus for a picture that never needed to leave the GPU. Through a dmabuf the
//! compositor draws into memory the renderer exported, and the frame is a
//! texture the moment it is done. A configuration asks for the second with
//! `{ gpu = true }`, and gets the first wherever the second cannot be had.

use morf_lua::{Runtime, Screencopy as LuaScreencopy};
use morf_render::{FOURCC_ARGB8888, FOURCC_XRGB8888, RenderEngine, WgpuBackend, split_dev_t};
use morf_wayland::{CaptureBuffer, LayerClient, LayerEvent, ScreencopyFormat};
use std::os::fd::AsFd;

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
            Some(identifier) => client.capture_window(request.id, identifier, request.gpu),
            None if request.include_cursor => client.capture_output(request.id, true),
            None => {
                client.capture_output_image(request.id, request.gpu)
                    || client.capture_output(request.id, false)
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
    mut renderer: Option<&mut RenderEngine<WgpuBackend>>,
    request_id: u64,
    result: Result<morf_wayland::ScreencopyFrame, String>,
) -> bool {
    // Published where `ui.Image` can find it, before the configuration is told
    // the capture arrived — so a handler can put the thumbnail straight into
    // its scene rather than being handed pixels with nowhere to go.
    //
    // Named by the request, so a thumbnail that refreshes replaces itself
    // instead of leaking an image per frame. A capture drawn on the GPU is
    // the texture that was exported for it, under `gpu:`; one that came
    // through shared memory is uploaded, under `memory:`.
    let name = match runtime.take_screencopy_name(request_id) {
        Some(name) => format!("capture/{name}"),
        None => format!("capture/{request_id}"),
    };
    let mut result = result;
    let mut source = format!("memory:{name}");
    // A capture that failed after an image was exported for it -- the
    // compositor refused the buffer, or the window went away -- would leave
    // that image waiting for a picture that is never coming.
    if let (Some(renderer), Err(_)) = (renderer.as_deref_mut(), &result) {
        renderer.backend_mut().take_export(request_id);
    }
    match (renderer, &mut result) {
        (Some(renderer), Ok(frame)) if frame.dmabuf => {
            let backend = renderer.backend_mut();
            let published = backend
                .take_export(request_id)
                .ok_or_else(|| "the capture arrived on the GPU with no image waiting".to_owned())
                .and_then(|image| backend.publish_texture(&name, image));
            match published {
                Ok(()) => {
                    source = format!("gpu:{name}");
                    if std::env::var_os("MORF_CAPTURE_READBACK").is_some()
                        && let Some(pixels) = backend.texture_pixels(&name)
                    {
                        // The one copy this path exists to avoid, made on
                        // request so a configuration can compare the two
                        // paths byte for byte; laid out as shared memory
                        // would have been, so the comparison is direct.
                        frame.stride = frame.width * 4;
                        frame.pixels = capture_bgra(&pixels.rgba);
                    }
                }
                Err(error) => result = Err(error),
            }
        }
        (Some(renderer), Ok(frame)) => {
            renderer.backend_mut().publish_image(
                name,
                frame.width,
                frame.height,
                capture_rgba(frame),
            );
        }
        (None, Ok(frame)) if frame.dmabuf => {
            result = Err("the capture arrived on the GPU where nothing can draw it".to_owned());
        }
        _ => {}
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
            gpu: frame.dmabuf,
            source,
            pixels: frame.pixels,
        }),
    )
}

/// Drops the published captures a configuration has released.
///
/// A source as `frame.source` gave it, with either prefix, or the bare
/// name: whichever side holds it lets go, and a name nothing held is not
/// an error, since a release after a replacement is the natural thing to
/// write.
pub(crate) fn apply_capture_releases(
    runtime: &mut Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
) {
    for source in runtime.take_screencopy_releases() {
        let name = source
            .strip_prefix("gpu:")
            .or_else(|| source.strip_prefix("memory:"))
            .unwrap_or(&source);
        let name = if name.starts_with("capture/") {
            name.to_owned()
        } else {
            format!("capture/{name}")
        };
        let backend = renderer.backend_mut();
        backend.forget_image(&name);
        backend.forget_texture(&name);
    }
}

/// Handles the two capture events, or hands any other event back.
pub(crate) fn handle_capture_event(
    runtime: &mut Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &mut LayerClient,
    event: LayerEvent,
) -> Result<bool, LayerEvent> {
    match event {
        LayerEvent::Screencopy { request_id, result } => Ok(dispatch_screencopy(
            runtime,
            Some(renderer),
            request_id,
            result,
        )),
        LayerEvent::CaptureOffer {
            request_id,
            width,
            height,
            device,
            formats,
        } => Ok(answer_capture_offer(
            runtime,
            Some(renderer),
            client,
            OfferedCapture {
                request_id,
                width,
                height,
                device,
                formats,
            },
        )),
        other => Err(other),
    }
}

/// Answers a compositor's offer to draw a capture into a dmabuf.
///
/// The renderer exports an image in a format and layout the compositor
/// named, on the device it named, and the file descriptor goes back as the
/// buffer to draw into. Any of that failing is not a failed capture: it is a
/// capture through shared memory, which is what the compositor would have
/// done had nobody asked.
pub(crate) fn answer_capture_offer(
    runtime: &mut Runtime,
    renderer: Option<&mut RenderEngine<WgpuBackend>>,
    client: &mut LayerClient,
    offer: OfferedCapture,
) -> bool {
    let why = match export_for_offer(renderer, client, &offer) {
        Ok(()) => return false,
        Err(why) => why,
    };
    if client.attach_capture_shm(offer.request_id) {
        return false;
    }
    runtime.dispatch_screencopy(offer.request_id, Err(why))
}

/// What a compositor said about a capture it will draw on the GPU.
pub(crate) struct OfferedCapture {
    pub(crate) request_id: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) device: Option<u64>,
    pub(crate) formats: Vec<(u32, Vec<u64>)>,
}

fn export_for_offer(
    renderer: Option<&mut RenderEngine<WgpuBackend>>,
    client: &mut LayerClient,
    offer: &OfferedCapture,
) -> Result<(), String> {
    let (request_id, width, height, device) =
        (offer.request_id, offer.width, offer.height, offer.device);
    let formats = &offer.formats;
    let backend = renderer
        .ok_or("no renderer here to draw a GPU capture")?
        .backend_mut();
    let support = backend
        .dmabuf_support()
        .ok_or("this GPU cannot export a dmabuf")?;
    // The buffer must be on the device the compositor draws with. A machine
    // with two GPUs may be rendering this shell on the other one, and a
    // buffer from there is memory the compositor cannot reach.
    if let (Some(device), Some(node)) = (device, support.render_node)
        && split_dev_t(device) != node
    {
        return Err("the compositor draws on another GPU".to_owned());
    }
    // The compositor's order of preference, filtered to what can be carried:
    // both formats are the same bytes, and only the alpha differs.
    let (fourcc, modifiers) = formats
        .iter()
        .find(|(fourcc, _)| *fourcc == FOURCC_XRGB8888 || *fourcc == FOURCC_ARGB8888)
        .ok_or("the compositor offered no format this engine draws")?;
    let image = backend.export_capture(width, height, *fourcc, modifiers)?;
    client
        .attach_capture_dmabuf(
            request_id,
            &CaptureBuffer {
                fd: image.plane.fd.as_fd(),
                width,
                height,
                fourcc: *fourcc,
                modifier: image.modifier,
                offset: image.plane.offset,
                stride: image.plane.stride,
            },
        )
        .map_err(|error| error.to_string())?;
    backend.stash_export(request_id, image);
    Ok(())
}

/// RGBA back to the little-endian `xrgb8888` bytes a capture arrives as.
fn capture_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    bgra
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
