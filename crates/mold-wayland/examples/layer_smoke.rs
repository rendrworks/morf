use mold_layout::{Geometry, TextAlignment, TextElide, Transform2D};
use mold_render::{
    DamageRect, DistanceFieldStyle, DrawCommand, DrawList, Gradient, RenderBackend,
    VerticalAlignment, WgpuBackend,
};
use mold_scene::{Color, Element, Scene};
use mold_wayland::{BarConfig, LayerClient, LayerEvent, OutputPowerMode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = BarConfig {
        output: std::env::var("MOLD_OUTPUT").ok(),
        ..BarConfig::default()
    };
    let mut client = LayerClient::connect(config)?;
    let idle_notify = client.set_idle_timeouts(&[600_000]);
    let output_power = client.set_output_power(OutputPowerMode::On);
    let clipboard = client.supports_clipboard();
    let virtual_keyboard = client.supports_virtual_keyboard();
    let input_method = client.supports_input_method();
    let text_input = client.supports_text_input();
    let screencopy = client.supports_screencopy();
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
                transform: Transform2D::IDENTITY,
                clip: None,
                color: Color::rgba8(31, 36, 48, 255),
                color_overlay: Color::rgba8(0, 0, 0, 0),
                gradient: Gradient::None,
                radii: [0.0; 4],
                border_width: 0.0,
                antialiasing: true,
                border_pixel_aligned: false,
                border_color: Color::rgba8(0, 0, 0, 0),
                blur: 0.0,
                shadow_color: Color::rgba8(0, 0, 0, 0),
                shadow_blur: 0.0,
                shadow_spread: 0.0,
                shadow_offset_x: 0.0,
                shadow_offset_y: 0.0,
                shadow_inner: false,
            },
            DrawCommand::Text {
                node: text_node,
                bounds: Geometry {
                    x: 12.0,
                    y: 4.0,
                    width: 240.0,
                    height: 24.0,
                },
                transform: Transform2D::IDENTITY,
                clip: None,
                text: "mold layer smoke".to_owned(),
                family: "sans-serif".to_owned(),
                font_source: String::new(),
                size: 18.0,
                font_weight: 400.0,
                color: Color::rgba8(255, 255, 255, 255),
                color_overlay: Color::rgba8(0, 0, 0, 0),
                wrap: false,
                elide: TextElide::None,
                horizontal_alignment: TextAlignment::Left,
                vertical_alignment: VerticalAlignment::Top,
                field_style: DistanceFieldStyle::default(),
            },
        ],
        layers: Vec::new(),
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
            if let LayerEvent::Frame { time_ms, .. } = event {
                let screens = client
                    .screens()
                    .iter()
                    .filter_map(|screen| screen.name.as_deref())
                    .collect::<Vec<_>>()
                    .join(",");
                println!(
                    "{}x{} at {}/120, screens [{}], idle {}, power {}, clipboard {}, keyboard {}, input-method {}, text-input {}, screencopy {}, frame {} ms, {} ({:?})",
                    client.logical_size().0,
                    client.logical_size().1,
                    client.scale_120(),
                    screens,
                    idle_notify,
                    output_power,
                    clipboard,
                    virtual_keyboard,
                    input_method,
                    text_input,
                    screencopy,
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
