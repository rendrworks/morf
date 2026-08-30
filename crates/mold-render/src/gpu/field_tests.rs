use super::*;
use crate::{SdfLayer, SdfOperation, SdfShapeKind};

/// Renders one command into a `size`-square target and reads the pixels back.
pub(super) fn render_readback(list: &DrawList, size: u32) -> Vec<u8> {
    let mut backend = pollster::block_on(WgpuBackend::new(size, size)).unwrap();
    backend
        .render(
            list,
            &[DamageRect {
                x: 0,
                y: 0,
                width: size,
                height: size,
            }],
            120,
        )
        .unwrap();
    let bytes_per_row = size.next_multiple_of(64) * 4;
    let buffer = backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mold field test readback"),
        size: u64::from(bytes_per_row) * u64::from(size),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = backend
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mold field test copy"),
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
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
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
    // Repack to a tight rgba grid so a caller can index by (x, y).
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for row in 0..size {
        let start = (row * bytes_per_row) as usize;
        out.extend_from_slice(&pixels[start..start + (size * 4) as usize]);
    }
    out
}

pub(super) fn field_layer(x: f64, y: f64, size: f64, shape: SdfShapeKind) -> SdfLayer {
    SdfLayer {
        bounds: Geometry {
            x,
            y,
            width: size,
            height: size,
        },
        color: Color::rgba8(255, 255, 255, 255),
        shape,
        morph_to: shape,
        morph: 0.0,
        operation: SdfOperation::Union,
        blend: 0.0,
        rotation: 0.0,
        radii: [0.0; 4],
        points: 5.0,
        inner_radius: 0.5,
        thickness: 4.0,
        angle: 90.0,
    }
}

pub(super) fn field_command(node: NodeHandle, layers: Vec<SdfLayer>) -> DrawCommand {
    DrawCommand::Field {
        node,
        bounds: Geometry {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
        },
        transform: Transform2D::IDENTITY,
        clip: None,
        fill_color: Color::rgba8(255, 255, 255, 255),
        stroke_color: Color::rgba8(0, 0, 0, 0),
        stroke_width: 0.0,
        softness: 0.0,
        layers,
    }
}

fn alpha_at(pixels: &[u8], size: u32, x: u32, y: u32) -> u8 {
    pixels[((y * size + x) * 4 + 3) as usize]
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_field_paints_its_shape_and_leaves_the_outside_clear() {
    // The shader compiles, the storage buffer binds, and the zero crossing
    // lands where the layer says it does.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let list = DrawList {
        commands: vec![field_command(
            node,
            vec![field_layer(16.0, 16.0, 32.0, SdfShapeKind::Circle)],
        )],
        layers: Vec::new(),
    };

    let pixels = render_readback(&list, 64);

    assert_eq!(alpha_at(&pixels, 64, 32, 32), 255, "inside the circle");
    assert_eq!(alpha_at(&pixels, 64, 2, 2), 0, "outside it");
    // The circle is inscribed in the layer box, so its own corners are outside.
    assert_eq!(alpha_at(&pixels, 64, 18, 18), 0, "the layer box corner");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_smooth_union_fills_the_gap_that_a_hard_union_leaves_open() {
    // The one thing a tessellated outline cannot do. Two circles with a gap
    // between them: a hard union leaves the midpoint empty, a smooth union with
    // a blend radius wide enough to bridge it fills it in.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let pair = |operation, blend| {
        let mut right = field_layer(36.0, 22.0, 20.0, SdfShapeKind::Circle);
        right.operation = operation;
        right.blend = blend;
        vec![field_layer(8.0, 22.0, 20.0, SdfShapeKind::Circle), right]
    };

    let hard = render_readback(
        &DrawList {
            commands: vec![field_command(node, pair(SdfOperation::Union, 0.0))],
            layers: Vec::new(),
        },
        64,
    );
    let smooth = render_readback(
        &DrawList {
            commands: vec![field_command(node, pair(SdfOperation::SmoothUnion, 24.0))],
            layers: Vec::new(),
        },
        64,
    );

    // Both circles are painted either way.
    assert_eq!(alpha_at(&hard, 64, 18, 32), 255, "left circle, hard");
    assert_eq!(alpha_at(&smooth, 64, 18, 32), 255, "left circle, smooth");
    // The midpoint between them is what the blend changes.
    assert_eq!(alpha_at(&hard, 64, 32, 32), 0, "gap stays open");
    assert_eq!(alpha_at(&smooth, 64, 32, 32), 255, "blend bridges the gap");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn subtracting_a_layer_opens_a_hole_through_the_one_before_it() {
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let mut hole = field_layer(24.0, 24.0, 16.0, SdfShapeKind::Circle);
    hole.operation = SdfOperation::Subtract;
    let list = DrawList {
        commands: vec![field_command(
            node,
            vec![field_layer(4.0, 4.0, 56.0, SdfShapeKind::Box), hole],
        )],
        layers: Vec::new(),
    };

    let pixels = render_readback(&list, 64);

    assert_eq!(alpha_at(&pixels, 64, 8, 8), 255, "the box remains");
    assert_eq!(alpha_at(&pixels, 64, 32, 32), 0, "the hole is open");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_morph_at_its_ends_matches_the_shape_at_each_end() {
    // A field morph is only trustworthy if it is an identity at zero and one.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let render = |shape, morph_to, morph| {
        let mut layer = field_layer(8.0, 8.0, 48.0, shape);
        layer.morph_to = morph_to;
        layer.morph = morph;
        render_readback(
            &DrawList {
                commands: vec![field_command(node, vec![layer])],
                layers: Vec::new(),
            },
            64,
        )
    };

    let circle = render(SdfShapeKind::Circle, SdfShapeKind::Circle, 0.0);
    let at_zero = render(SdfShapeKind::Circle, SdfShapeKind::Box, 0.0);
    let boxed = render(SdfShapeKind::Box, SdfShapeKind::Box, 0.0);
    let at_one = render(SdfShapeKind::Circle, SdfShapeKind::Box, 1.0);

    assert_eq!(circle, at_zero, "morph 0 is the start shape");
    assert_eq!(boxed, at_one, "morph 1 is the end shape");
    // And the two ends really are different shapes, so the test can fail.
    assert_ne!(
        alpha_at(&circle, 64, 10, 10),
        alpha_at(&boxed, 64, 10, 10),
        "a box fills the corner a circle does not"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_morph_between_one_shape_and_two_passes_through_a_split() {
    // The reason for interpolating fields rather than outlines: halfway between
    // one blob and two, the field describes a shape that is neither, and the
    // count of separate pieces changes without any correspondence between the
    // two ends.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    // One wide capsule, morphing towards a narrow one, beside a second circle.
    let render = |morph| {
        let mut left = field_layer(4.0, 22.0, 24.0, SdfShapeKind::Circle);
        left.morph_to = SdfShapeKind::Circle;
        let mut right = field_layer(36.0, 22.0, 24.0, SdfShapeKind::Circle);
        right.operation = SdfOperation::SmoothUnion;
        right.blend = morph;
        render_readback(
            &DrawList {
                commands: vec![field_command(node, vec![left, right])],
                layers: Vec::new(),
            },
            64,
        )
    };

    let apart = render(0.0);
    let joined = render(24.0);

    assert_eq!(alpha_at(&apart, 64, 32, 34), 0, "two separate pieces");
    assert_eq!(alpha_at(&joined, 64, 32, 34), 255, "one joined piece");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn every_shape_family_paints_something_and_stays_inside_its_layer() {
    // A shape whose distance function has the sign inverted, or whose scale is
    // wrong, shows up here as an empty layer or as one that floods the target.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    for shape in [
        SdfShapeKind::Circle,
        SdfShapeKind::Box,
        SdfShapeKind::Capsule,
        SdfShapeKind::Triangle,
        SdfShapeKind::Hexagon,
        SdfShapeKind::Star,
        SdfShapeKind::Ring,
        SdfShapeKind::Pie,
        SdfShapeKind::Cross,
    ] {
        let pixels = render_readback(
            &DrawList {
                commands: vec![field_command(
                    node,
                    vec![field_layer(16.0, 16.0, 32.0, shape)],
                )],
                layers: Vec::new(),
            },
            64,
        );
        let painted = (0..64 * 64)
            .filter(|index| pixels[index * 4 + 3] > 128)
            .count();
        assert!(painted > 16, "{shape:?} painted almost nothing: {painted}");
        assert!(
            painted < 64 * 64 / 2,
            "{shape:?} flooded the target: {painted}"
        );
        // Nothing may reach the corners of a 64-square target from a 32-square
        // layer centred in it.
        assert_eq!(alpha_at(&pixels, 64, 0, 0), 0, "{shape:?} leaked");
        assert_eq!(alpha_at(&pixels, 64, 63, 63), 0, "{shape:?} leaked");
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_fractional_point_count_grows_a_star_point_instead_of_popping_it_in() {
    // A star is only defined for a whole number of points. Rounding the count
    // makes a new spike appear at full size between two frames; blending the
    // neighbouring stars as fields grows it out of the edge instead.
    //
    // Area is the wrong thing to measure — the blended field is genuinely a
    // different shape, and narrower than either end. What matters is that the
    // change is spread across the sweep rather than concentrated in one step,
    // which is exactly the difference between growing a point and popping it in.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let at = |points: f32| {
        let mut layer = field_layer(8.0, 8.0, 48.0, SdfShapeKind::Star);
        layer.points = points;
        layer.inner_radius = 0.45;
        render_readback(
            &DrawList {
                commands: vec![field_command(node, vec![layer])],
                layers: Vec::new(),
            },
            64,
        )
    };
    /// How many pixels changed coverage between two renders.
    fn moved(a: &[u8], b: &[u8]) -> usize {
        (0..64 * 64)
            .filter(|index| (a[index * 4 + 3] > 128) != (b[index * 4 + 3] > 128))
            .count()
    }

    let frames: Vec<_> = (0..=8).map(|step| at(5.0 + step as f32 / 8.0)).collect();
    let steps: Vec<_> = frames
        .windows(2)
        .map(|pair| moved(&pair[0], &pair[1]))
        .collect();
    let total: usize = steps.iter().sum();
    let largest = *steps.iter().max().expect("eight steps");

    assert!(total > 0, "the sweep changed nothing at all");
    // Rounding puts every pixel of the change into one step. Blending spreads
    // it, so no single step may account for most of the sweep.
    assert!(
        largest * 2 < total,
        "one step moved {largest} of {total} pixels: {steps:?}"
    );
}
