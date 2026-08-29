use mold_layout::Geometry;
use mold_render::{DamageRect, DrawCommand, DrawList, RenderBackend, WgpuBackend};
use mold_scene::{Color, Element, Scene};
use mold_wayland::{BarConfig, LayerClient, LayerEvent};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = LayerClient::connect(BarConfig::default())?;
    while !matches!(client.next_event(), Some(LayerEvent::Configure { .. })) {
        client.dispatch()?;
    }
    let (width, height) = client.physical_size();
    let mut backend = pollster::block_on(WgpuBackend::new_surface(
        client.window_target(),
        width,
        height,
    ))?;
    let mut scene = Scene::new();
    let node = scene.create(Element::Rect);
    let list = DrawList {
        commands: vec![DrawCommand::Quad {
            node,
            bounds: Geometry {
                x: 0.0,
                y: 0.0,
                width: client.logical_size().0 as f64,
                height: client.logical_size().1 as f64,
            },
            color: Color::rgba8(31, 36, 48, 255),
            radius: 0.0,
            border_width: 0.0,
            border_color: Color::rgba8(0, 0, 0, 0),
        }],
    };
    client.request_frame();
    client
        .surface()
        .damage_buffer(0, 0, width as i32, height as i32);
    backend.render(
        &list,
        &[DamageRect {
            x: 0,
            y: 0,
            width,
            height,
        }],
        client.scale_120(),
    )?;
    loop {
        client.dispatch()?;
        if let Some(LayerEvent::Frame { time_ms }) = client.next_event() {
            println!(
                "{}x{} at {}/120, frame {} ms, {} ({:?})",
                client.logical_size().0,
                client.logical_size().1,
                client.scale_120(),
                time_ms,
                backend.info().name,
                backend.info().backend,
            );
            break;
        }
    }
    Ok(())
}
