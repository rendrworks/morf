use mold_layout::{Geometry, TextAlignment, TextElide, Transform2D};
use mold_render::{
    DamageRect, DrawCommand, DrawList, Gradient, ImageFillMode, Layer, LayerMask, RenderBackend,
    VerticalAlignment, WgpuBackend,
};
use mold_scene::{Color, Element, Scene};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image_path =
        std::env::temp_dir().join(format!("mold-gpu-smoke-{}.svg", std::process::id()));
    std::fs::write(
        &image_path,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><circle cx="16" cy="16" r="14" fill="#ffffff"/></svg>"##,
    )?;
    let mut backend = pollster::block_on(WgpuBackend::new(320, 64))?;
    let mut scene = Scene::new();
    let node = scene.create(Element::Rect);
    let image = scene.create(Element::Image);
    let shape = scene.create(Element::Shape);
    let text = scene.create(Element::Text);
    let list = DrawList {
        commands: vec![
            DrawCommand::Quad {
                node,
                bounds: Geometry {
                    x: 8.0,
                    y: 8.0,
                    width: 304.0,
                    height: 48.0,
                },
                transform: Transform2D::around((60.0, 35.0), 1.0, 7.5),
                clip: Some(Geometry {
                    x: 4.0,
                    y: 4.0,
                    width: 312.0,
                    height: 56.0,
                }),
                color: Color::rgba8(38, 115, 217, 255),
                color_overlay: Color::rgba8(0, 0, 0, 0),
                gradient: Gradient::Linear {
                    start_color: Color::rgba8(38, 115, 217, 255),
                    end_color: Color::rgba8(124, 58, 237, 255),
                    start: [0.0, 0.0],
                    end: [1.0, 0.0],
                },
                radii: [8.0, 16.0, 8.0, 16.0],
                border_width: 1.0,
                border_color: Color::rgba8(255, 255, 255, 128),
                blur: 0.0,
                shadow_color: Color::rgba8(0, 0, 0, 128),
                shadow_blur: 6.0,
                shadow_spread: 1.0,
                shadow_offset_x: 0.0,
                shadow_offset_y: 2.0,
            },
            DrawCommand::Texture {
                node: image,
                bounds: Geometry {
                    x: 144.0,
                    y: 16.0,
                    width: 32.0,
                    height: 32.0,
                },
                transform: Transform2D::IDENTITY,
                clip: None,
                source: image_path.to_string_lossy().into_owned(),
                icon_theme: None,
                opacity: 1.0,
                color_overlay: Color::rgba8(0, 0, 0, 0),
                fill_mode: ImageFillMode::PreserveAspectFit,
            },
            DrawCommand::Path {
                node: shape,
                bounds: Geometry {
                    x: 24.0,
                    y: 18.0,
                    width: 28.0,
                    height: 28.0,
                },
                transform: Transform2D::IDENTITY,
                clip: None,
                path: "M14 0 L28 28 L0 28 Z".to_owned(),
                fill_color: Color::rgba8(255, 255, 255, 220),
                stroke_color: Color::rgba8(0, 0, 0, 255),
                stroke_width: 1.0,
                even_odd: false,
            },
            DrawCommand::Text {
                node: text,
                bounds: Geometry {
                    x: 190.0,
                    y: 18.0,
                    width: 110.0,
                    height: 28.0,
                },
                transform: Transform2D::IDENTITY,
                clip: None,
                text: "mold".to_owned(),
                family: "sans-serif".to_owned(),
                size: 18.0,
                color: Color::rgba8(255, 255, 255, 255),
                color_overlay: Color::rgba8(0, 0, 0, 0),
                wrap: false,
                elide: TextElide::None,
                horizontal_alignment: TextAlignment::Left,
                vertical_alignment: VerticalAlignment::Center,
            },
        ],
        layers: vec![Layer {
            node,
            commands: 0..4,
            parent: None,
            opacity: 0.8,
            blur: 6.0,
            shadow_color: Color::rgba8(0, 0, 0, 160),
            shadow_blur: 8.0,
            shadow_offset: [3.0, 4.0],
            mask: Some(LayerMask {
                bounds: Geometry {
                    x: 0.0,
                    y: 0.0,
                    width: 320.0,
                    height: 64.0,
                },
                transform: Transform2D::IDENTITY,
                radii: [12.0; 4],
            }),
            bounds: Geometry {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 64.0,
            },
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
    std::fs::remove_file(image_path)?;
    Ok(())
}
