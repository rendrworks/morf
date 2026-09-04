use morf_layout::{Geometry, Transform2D};
use morf_render::{DamageRect, DrawCommand, DrawList, RenderBackend, WgpuBackend};
use morf_scene::{Color, Element, Scene};
use morf_wayland::{BarConfig, InputRect, LayerClient, LayerEvent, PopupConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = LayerClient::connect(BarConfig::default())?;
    wait_for_layer(&mut client)?;
    let (bar_width, bar_height) = client.physical_size();
    let mut bar = pollster::block_on(WgpuBackend::new_surface(
        client.window_target(),
        bar_width,
        bar_height,
    ))?;
    let mut scene = Scene::new();
    let bar_node = scene.create(Element::Rect);
    bar.render(
        &DrawList {
            commands: vec![quad(
                bar_node,
                0.0,
                0.0,
                client.logical_size().0 as f64,
                client.logical_size().1 as f64,
                Color::rgba8(31, 36, 48, 255),
            )],
            layers: Vec::new(),
        },
        &[DamageRect {
            x: 0,
            y: 0,
            width: bar_width,
            height: bar_height,
        }],
        client.scale_120(),
    )?;

    let (x, y) = popup_anchor(&mut client)?;
    client.open_popup(
        0,
        morf_wayland::SurfaceRole::Layer(0),
        PopupConfig {
            anchor: InputRect {
                x: x.floor() as i32,
                y: y.floor() as i32,
                width: 1,
                height: 1,
            },
            width: 240,
            height: 120,
            ..PopupConfig::default()
        },
    )?;
    let (width, height) = wait_for_popup(&mut client)?;
    let scale = client.scale_120();
    let physical_width = ((width as u64 * scale as u64).div_ceil(120)) as u32;
    let physical_height = ((height as u64 * scale as u64).div_ceil(120)) as u32;
    let mut popup = pollster::block_on(WgpuBackend::new_surface(
        client.popup_window_target(0).ok_or("popup was dismissed")?,
        physical_width,
        physical_height,
    ))?;
    let popup_node = scene.create(Element::Rect);
    client.request_popup_frame(0);
    client
        .popup_surface(0)
        .ok_or("popup was dismissed")?
        .damage_buffer(0, 0, physical_width as i32, physical_height as i32);
    popup.render(
        &DrawList {
            commands: vec![quad(
                popup_node,
                0.0,
                0.0,
                width as f64,
                height as f64,
                Color::rgba8(66, 76, 96, 255),
            )],
            layers: Vec::new(),
        },
        &[DamageRect {
            x: 0,
            y: 0,
            width: physical_width,
            height: physical_height,
        }],
        scale,
    )?;
    'framed: loop {
        client.dispatch()?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::PopupFrame { time_ms, .. } => {
                    // The popup's own scale, which is the point: it used to
                    // be given the primary layer's, and on a mixed-DPI desk
                    // those differ.
                    let popup_scale = client.surface_scale_120(morf_wayland::SurfaceRole::Popup(0));
                    println!(
                        "click-anchored popup {width}x{height} at {popup_scale}/120 \
                         (layer {scale}/120), frame {time_ms} ms"
                    );
                    break 'framed;
                }
                LayerEvent::PopupDone { .. } => return Err("popup was dismissed".into()),
                _ => {}
            }
        }
    }
    Ok(())
}

fn wait_for_layer(client: &mut LayerClient) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        client.dispatch()?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::Configure { .. } => return Ok(()),
                LayerEvent::Closed { .. } => return Err("layer surface was closed".into()),
                _ => {}
            }
        }
    }
}

fn popup_anchor(client: &mut LayerClient) -> Result<(f64, f64), Box<dyn std::error::Error>> {
    if std::env::var_os("MORF_POPUP_AUTO").is_some() {
        return Ok((16.0, client.logical_size().1 as f64));
    }
    loop {
        client.dispatch()?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::PointerButton {
                    pressed: true,
                    x,
                    y,
                    ..
                } => return Ok((x, y)),
                LayerEvent::Closed { .. } => return Err("layer surface was closed".into()),
                _ => {}
            }
        }
    }
}

fn wait_for_popup(client: &mut LayerClient) -> Result<(u32, u32), Box<dyn std::error::Error>> {
    loop {
        client.dispatch()?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::PopupConfigure { width, height, .. } => return Ok((width, height)),
                LayerEvent::PopupDone { .. } => return Err("popup was dismissed".into()),
                _ => {}
            }
        }
    }
}

fn quad(
    node: morf_scene::NodeHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    color: Color,
) -> DrawCommand {
    DrawCommand::Quad {
        node,
        bounds: Geometry {
            x,
            y,
            width,
            height,
        },
        transform: Transform2D::IDENTITY,
        clip: None,
        color,
        color_overlay: Color::rgba8(0, 0, 0, 0),
        gradient: None,
        radii: [8.0; 4],
        border_width: 1.0,
        antialiasing: true,
        border_pixel_aligned: false,
        border_color: Color::rgba8(255, 255, 255, 80),
        blur: 0.0,
        shadow_color: Color::rgba8(0, 0, 0, 100),
        shadow_blur: 8.0,
        shadow_spread: 0.0,
        shadow_offset_x: 0.0,
        shadow_offset_y: 3.0,
        shadow_inner: false,
        shader: None,
    }
}
