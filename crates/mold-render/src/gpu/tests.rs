use super::*;

fn test_quad(
    node: NodeHandle,
    color: Color,
    border_color: Color,
    border_width: f64,
) -> DrawCommand {
    DrawCommand::Quad {
        node,
        bounds: Geometry {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 4.0,
        },
        transform: Transform2D::IDENTITY,
        clip: None,
        color,
        color_overlay: Color::rgba8(0, 0, 0, 0),
        gradient: crate::Gradient::None,
        radii: [0.0; 4],
        border_width,
        antialiasing: false,
        border_pixel_aligned: true,
        border_color,
        blur: 0.0,
        shadow_color: Color::rgba8(0, 0, 0, 0),
        shadow_blur: 0.0,
        shadow_spread: 0.0,
        shadow_offset_x: 0.0,
        shadow_offset_y: 0.0,
        shadow_inner: false,
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn srgb_target_preserves_hex_colors_and_blends_borders() {
    let mut backend = pollster::block_on(WgpuBackend::new(4, 4)).unwrap();
    let mut scene = mold_scene::Scene::new();
    let background = scene.create(mold_scene::Element::Rect);
    let border = scene.create(mold_scene::Element::Rect);
    let list = DrawList {
        commands: vec![
            test_quad(
                background,
                Color::rgba8(33, 34, 41, 255),
                Color::rgba8(0, 0, 0, 0),
                0.0,
            ),
            test_quad(
                border,
                Color::rgba8(0, 0, 0, 0),
                Color::rgba8(190, 198, 240, 20),
                1.0,
            ),
        ],
        layers: Vec::new(),
    };
    backend
        .render(
            &list,
            &[DamageRect {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            }],
            120,
        )
        .unwrap();

    let bytes_per_row = 256;
    let buffer = backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mold color test readback"),
        size: bytes_per_row * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = backend
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mold color test copy"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &backend.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row as u32),
                rows_per_image: Some(4),
            },
        },
        wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    backend.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (send, receive) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        send.send(result).unwrap()
    });
    backend
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    receive.recv().unwrap().unwrap();
    let pixels = slice.get_mapped_range().unwrap();

    assert_eq!(&pixels[0..4], &[66, 69, 84, 255]);
    let center = 2 * bytes_per_row as usize + 2 * 4;
    assert_eq!(&pixels[center..center + 4], &[33, 34, 41, 255]);
}

#[test]
fn scissor_is_clamped_to_the_physical_target() {
    assert_eq!(
        clamp_scissor(
            DamageRect {
                x: 8,
                y: 9,
                width: 20,
                height: 20,
            },
            10,
            12,
        ),
        Some((8, 9, 2, 3))
    );
}

#[test]
fn damage_and_clip_scissors_are_intersected() {
    assert_eq!(
        intersect_damage(
            DamageRect {
                x: 0,
                y: 10,
                width: 40,
                height: 20,
            },
            DamageRect {
                x: 20,
                y: 0,
                width: 30,
                height: 20,
            },
        ),
        Some(DamageRect {
            x: 20,
            y: 10,
            width: 20,
            height: 10,
        })
    );
}

#[test]
fn texture_fit_and_crop_preserve_aspect_ratio() {
    let bounds = Geometry {
        x: 10.0,
        y: 20.0,
        width: 100.0,
        height: 100.0,
    };
    let fit = texture_placement(
        bounds,
        (200, 100),
        ImageFillMode::PreserveAspectFit,
        Transform2D::IDENTITY,
    );
    assert_eq!(
        fit.bounds,
        Geometry {
            x: 10.0,
            y: 45.0,
            width: 100.0,
            height: 50.0
        }
    );
    assert_eq!(fit.uv, [0.0, 0.0, 1.0, 1.0]);

    let crop = texture_placement(
        bounds,
        (200, 100),
        ImageFillMode::PreserveAspectCrop,
        Transform2D::IDENTITY,
    );
    assert_eq!(crop.bounds, bounds);
    assert_eq!(crop.logical_width, 200);
    assert_eq!(crop.logical_height, 100);
    assert_eq!(crop.uv, [0.25, 0.0, 0.5, 1.0]);
}

#[test]
fn layer_mask_data_inverts_the_owner_transform() {
    let mask = LayerMask {
        bounds: Geometry {
            x: 10.0,
            y: 20.0,
            width: 40.0,
            height: 30.0,
        },
        transform: Transform2D::around((10.0, 20.0), 2.0, 0.0),
        radii: [4.0, 5.0, 6.0, 7.0],
    };

    let (enabled, bounds, inverse_0, inverse_1, radii) = layer_mask_data(Some(mask));

    assert_eq!(enabled, 1.0);
    assert_eq!(bounds, [10.0, 20.0, 40.0, 30.0]);
    assert_eq!(inverse_0, [0.5, 0.0, 5.0, 0.0]);
    assert_eq!(inverse_1, [0.0, 0.5, 10.0, 0.0]);
    assert_eq!(radii, [4.0, 5.0, 6.0, 7.0]);
}

#[test]
fn glyph_shelves_reserve_padding_and_wrap_rows() {
    let mut allocator = ShelfAllocator::default();

    assert_eq!(allocator.allocate(1022, 10), Some((1, 1)));
    assert_eq!(allocator.allocate(1022, 20), Some((1025, 1)));
    assert_eq!(allocator.allocate(1, 1), Some((1, 23)));
}
