use crate::*;

// The two halves of the one shape vocabulary, held against each other.
//
// `mold_region::distance` decides where a click lands; `field.wgsl` decides
// where a pixel is painted. They are ports of one another, and nothing but a
// test keeps a port honest — a family whose discriminant, parameter slot or
// arithmetic drifts on one side renders in one place and is clickable in
// another, which is exactly the bug that having two vocabularies used to
// guarantee.

use crate::gpu::field_tests::{alpha_at, field_command, field_layer, read_frame, render_readback};
use crate::{Operation, Shape, ShapeParams};
use mold_scene::Color;

const SIZE: u32 = 64;
/// How far from the analytic edge a pixel has to be before its coverage is
/// allowed to be asserted. Inside this band the shader is antialiasing and the
/// CPU is answering a yes-or-no question, so the two legitimately differ.
const MARGIN: f32 = 2.0;

fn params_for(shape: Shape) -> ShapeParams {
    match shape {
        Shape::Box => ShapeParams::rounded(8.0),
        // A default four-pixel wall is thinner than the margin either side of
        // it, so nothing would ever be far enough inside a ring or a cross arm
        // to assert on. Twelve gives the sweep something to hold.
        Shape::Ring | Shape::Cross => ShapeParams {
            thickness: 12.0,
            ..ShapeParams::default()
        },
        _ => ShapeParams::default(),
    }
}

/// Every family in the vocabulary, drawn one at a time.
fn families() -> [Shape; 10] {
    [
        Shape::Circle,
        Shape::Box,
        Shape::Capsule,
        Shape::Triangle,
        Shape::Hexagon,
        Shape::Star,
        Shape::Ring,
        Shape::Pie,
        Shape::Cross,
        Shape::Ellipse,
    ]
}

/// The draw list for one family, alone in its own field.
fn one(shape: Shape, inset: f64, width: f64, height: f64) -> DrawList {
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let params = params_for(shape);
    let mut layer = field_layer(inset, inset, width, shape);
    layer.bounds.height = height;
    layer.operation = Operation::Union;
    // Every parameter, not just the corner radii: a family whose thickness or
    // point count reached the shader as the helper's default while the CPU saw
    // the test's would disagree for a reason that has nothing to do with the
    // shapes.
    layer.radii = params.radii;
    layer.points = params.points;
    layer.inner_radius = params.inner_radius;
    layer.thickness = params.thickness;
    layer.angle = params.angle;
    DrawList {
        commands: vec![field_command(node, vec![layer])],
        layers: Vec::new(),
    }
}

/// Half-extents and centre in the same frame the CPU distance function wants.
fn frame(inset: f64, width: f64, height: f64) -> ([f32; 2], [f32; 2]) {
    (
        [(width / 2.0) as f32, (height / 2.0) as f32],
        [(inset + width / 2.0) as f32, (inset + height / 2.0) as f32],
    )
}

/// Renders one family through a backend that is reused across the whole sweep.
///
/// Ten families times two box shapes is twenty frames, and standing up a fresh
/// `WgpuBackend` — a fresh device — for each of them is enough to take the
/// driver down. One device, twenty frames.
fn draw(backend: &mut WgpuBackend, shape: Shape, inset: f64, width: f64, height: f64) -> Vec<u8> {
    read_frame(backend, &one(shape, inset, width, height), SIZE)
}

/// Asserts that every pixel further than `MARGIN` from the analytic edge is
/// painted exactly as the CPU says it should be.
fn agrees(shape: Shape, pixels: &[u8], half: [f32; 2], centre: [f32; 2]) {
    let params = params_for(shape);
    let mut inside = 0;
    let mut outside = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let point = [x as f32 + 0.5 - centre[0], y as f32 + 0.5 - centre[1]];
            let signed = mold_region::distance(shape, &params, half, point);
            let alpha = alpha_at(pixels, SIZE, x, y);
            if signed < -MARGIN {
                inside += 1;
                assert_eq!(
                    alpha,
                    255,
                    "{} is {signed:.2}px inside at ({x}, {y}) but the shader left it clear",
                    shape.name(),
                );
            } else if signed > MARGIN {
                outside += 1;
                assert_eq!(
                    alpha,
                    0,
                    "{} is {signed:.2}px outside at ({x}, {y}) but the shader painted it",
                    shape.name(),
                );
            }
        }
    }
    // A family whose distance function came back all-positive or all-negative
    // would satisfy every assertion above by never reaching one.
    // A thin family in a flat box — a star at a twelve-pixel radius — has few
    // pixels more than the margin from its own edge, so this is only a guard
    // against a distance function that came back uniformly signed, not a
    // coverage target.
    assert!(
        inside >= 8,
        "{} covered only {inside} pixels — the test proved nothing",
        shape.name(),
    );
    assert!(
        outside >= 8,
        "{} left only {outside} pixels clear — the test proved nothing",
        shape.name(),
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn every_family_paints_where_the_region_rasteriser_says_it_will() {
    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    let (half, centre) = frame(8.0, 48.0, 48.0);
    for shape in families() {
        let pixels = draw(&mut backend, shape, 8.0, 48.0, 48.0);
        agrees(shape, &pixels, half, centre);
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn every_family_agrees_in_a_box_that_is_not_square() {
    // A non-square box is where the two used to part company: the renderer's
    // circle inscribes the shorter side while an input region's ellipse
    // stretches to the box, and having only one of those words meant the
    // difference could not even be expressed.
    let mut backend = pollster::block_on(WgpuBackend::new(SIZE, SIZE)).unwrap();
    let (half, centre) = frame(8.0, 48.0, 24.0);
    for shape in families() {
        let pixels = draw(&mut backend, shape, 8.0, 48.0, 24.0);
        agrees(shape, &pixels, half, centre);
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_circle_and_an_ellipse_are_different_shapes_in_a_wide_box() {
    // The distinction the merge had to preserve rather than collapse. If these
    // two ever render the same, one of the words has quietly stopped meaning
    // anything.
    let circle = render_readback(&one(Shape::Circle, 8.0, 48.0, 24.0), SIZE);
    let ellipse = render_readback(&one(Shape::Ellipse, 8.0, 48.0, 24.0), SIZE);
    // Mid-height, near the left end of the wide box: inside the stretched
    // ellipse, outside the inscribed circle.
    let x = 12;
    let y = 20;
    assert_eq!(
        alpha_at(&ellipse, SIZE, x, y),
        255,
        "the ellipse fills the width of its box",
    );
    assert_eq!(
        alpha_at(&circle, SIZE, x, y),
        0,
        "the circle stays inscribed in the shorter side",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn xor_leaves_the_overlap_open() {
    // Xor was the input region's word and the shader had no case for it, so a
    // configuration could compose one only on the half of the vocabulary that
    // could not draw. Two overlapping squares: both ends painted, the shared
    // middle clear.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let left = field_layer(4.0, 20.0, 32.0, Shape::Box);
    let mut right = field_layer(28.0, 20.0, 32.0, Shape::Box);
    right.operation = Operation::Xor;
    let list = DrawList {
        commands: vec![field_command(node, vec![left, right])],
        layers: Vec::new(),
    };

    let pixels = render_readback(&list, SIZE);

    assert_eq!(
        alpha_at(&pixels, SIZE, 10, 34),
        255,
        "the left square is painted"
    );
    assert_eq!(alpha_at(&pixels, SIZE, 54, 34), 255, "and the right one");
    assert_eq!(
        alpha_at(&pixels, SIZE, 32, 34),
        0,
        "and the overlap they share is taken back out",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_composed_shape_can_carry_a_gradient_like_a_rectangle() {
    // What the pipeline merge is for. A gradient, a border and a shadow used to
    // belong to the quad pipeline, and a star was not drawn by the quad
    // pipeline — so a star could be any one flat colour and nothing else. One
    // pipeline draws both now, and the material is the same material.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let mut command = field_command(node, vec![field_layer(8.0, 8.0, 48.0, Shape::Star)]);
    let DrawCommand::Field {
        gradient,
        fill_color,
        ..
    } = &mut command
    else {
        panic!("field_command builds a field");
    };
    *fill_color = Color::rgba8(255, 255, 255, 255);
    *gradient = Gradient::Linear {
        start_color: Color::rgba8(255, 0, 0, 255),
        end_color: Color::rgba8(0, 0, 255, 255),
        start: [0.0, 0.0],
        end: [1.0, 0.0],
    };
    let list = DrawList {
        commands: vec![command],
        layers: Vec::new(),
    };

    let pixels = render_readback(&list, SIZE);

    // Read across the star's waist, inside the shape on both sides of centre.
    let red_at = |x: u32, y: u32| pixels[((y * SIZE + x) * 4) as usize];
    let blue_at = |x: u32, y: u32| pixels[((y * SIZE + x) * 4 + 2) as usize];
    let left = (26, 30);
    let right = (38, 30);
    assert_eq!(
        alpha_at(&pixels, SIZE, left.0, left.1),
        255,
        "left is inside"
    );
    assert_eq!(
        alpha_at(&pixels, SIZE, right.0, right.1),
        255,
        "right is inside",
    );
    assert!(
        red_at(left.0, left.1) > red_at(right.0, right.1),
        "the gradient starts red on the left",
    );
    assert!(
        blue_at(right.0, right.1) > blue_at(left.0, left.1),
        "and ends blue on the right",
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_composed_shape_casts_a_shadow_shaped_like_itself() {
    // A shadow used to be a second rounded box, grown by the spread and offset.
    // It is the composition itself now — moved, and dilated by subtracting the
    // spread from its distance — so a star's shadow is star-shaped.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let mut command = field_command(node, vec![field_layer(6.0, 6.0, 36.0, Shape::Star)]);
    let DrawCommand::Field {
        shadow_color,
        shadow_offset_x,
        shadow_offset_y,
        ..
    } = &mut command
    else {
        panic!("field_command builds a field");
    };
    *shadow_color = Color::rgba8(0, 0, 0, 255);
    *shadow_offset_x = 12.0;
    *shadow_offset_y = 12.0;
    let list = DrawList {
        commands: vec![command],
        layers: Vec::new(),
    };

    let pixels = render_readback(&list, SIZE);

    // The star's centre, and the same point moved by the shadow offset.
    assert_eq!(alpha_at(&pixels, SIZE, 24, 24), 255, "the star is drawn");
    assert_eq!(
        alpha_at(&pixels, SIZE, 36, 36),
        255,
        "and its shadow is under the offset copy of it",
    );
    // A corner the star does not reach, and its shadow does not either. A
    // bounding-box shadow would cover this.
    assert_eq!(
        alpha_at(&pixels, SIZE, 19, 55),
        0,
        "the shadow is star-shaped, not box-shaped",
    );
    // The layer itself ends at 42; anything painted past that is shadow, and
    // there is none at all unless the quad was widened to make room for it.
    let beyond = (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .filter(|(x, y)| *x > 43 || *y > 43)
        .filter(|(x, y)| alpha_at(&pixels, SIZE, *x, *y) > 0)
        .count();
    assert!(
        beyond > 40,
        "the quad grew to hold the shadow instead of clipping it: {beyond} pixels past the layer",
    );
}
