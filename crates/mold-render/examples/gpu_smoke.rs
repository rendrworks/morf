use mold_layout::Geometry;
use mold_render::{DamageRect, DrawCommand, DrawList, RenderBackend, WgpuBackend};
use mold_scene::{Color, Element, Scene};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut backend = pollster::block_on(WgpuBackend::new(320, 64))?;
    let mut scene = Scene::new();
    let node = scene.create(Element::Rect);
    let list = DrawList {
        commands: vec![DrawCommand::Quad {
            node,
            bounds: Geometry {
                x: 8.0,
                y: 8.0,
                width: 304.0,
                height: 48.0,
            },
            color: Color::rgba8(38, 115, 217, 255),
            radius: 8.0,
            border_width: 1.0,
            border_color: Color::rgba8(255, 255, 255, 128),
        }],
    };
    backend.render(
        &list,
        &[DamageRect {
            x: 0,
            y: 0,
            width: 320,
            height: 64,
        }],
        120,
    )?;
    let info = backend.info();
    println!(
        "{} ({:?}, {:04x}:{:04x})",
        info.name, info.backend, info.vendor, info.device
    );
    Ok(())
}
