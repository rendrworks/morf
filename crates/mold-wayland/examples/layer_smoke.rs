use mold_layout::Geometry;
use mold_render::{DamageRect, DrawCommand, DrawList, RenderBackend, WgpuBackend};
use mold_scene::{Color, Element, Scene};
use mold_wayland::{BarConfig, LayerClient, LayerEvent};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = LayerClient::connect(BarConfig::default())?;
    'configured: loop {
        client.dispatch()?;
        while let Some(event) = client.next_event() {
            if matches!(event, LayerEvent::Configure { .. }) {
                break 'configured;
            }
        }
    }
    let (width, height) = client.physical_size();
    let mut backend = pollster::block_on(WgpuBackend::new_surface(
        client.window_target(),
        width,
        height,
    ))?;
    let mut scene = Scene::new();
    let node = scene.create(Element::Rect);
    let text_node = scene.create(Element::Text);
    let list = DrawList {
        commands: vec![
            DrawCommand::Quad {
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
            },
            DrawCommand::Text {
                node: text_node,
                bounds: Geometry {
                    x: 12.0,
                    y: 4.0,
                    width: 240.0,
                    height: 24.0,
                },
                text: "mold layer smoke".to_owned(),
                family: "sans-serif".to_owned(),
                size: 18.0,
                color: Color::rgba8(255, 255, 255, 255),
            },
        ],
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
    'framed: loop {
        client.dispatch()?;
        while let Some(event) = client.next_event() {
            if let LayerEvent::Frame { time_ms } = event {
                println!(
                    "{}x{} at {}/120, {} screens, frame {} ms, {} ({:?})",
                    client.logical_size().0,
                    client.logical_size().1,
                    client.scale_120(),
                    client.screens().len(),
                    time_ms,
                    backend.info().name,
                    backend.info().backend,
                );
                break 'framed;
            }
        }
    }
    Ok(())
}
