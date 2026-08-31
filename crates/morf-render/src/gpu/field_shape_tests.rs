use super::*;
use crate::*;

// Shapes a configuration actually asks for, drawn end to end: a window frame,
// an oversized corner radius, a rotated bar.

use crate::gpu::field_tests::{alpha_at, field_command, field_layer, read_frame, render_readback};
use crate::{Operation, Shape};

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_frame_is_one_box_with_another_taken_out_of_the_middle() {
    // The screen border, which used to be an SVG path with four arc commands
    // in it and a tessellator to turn them into triangles. As a field it is the
    // output rectangle, minus the visible area rounded off — two layers and one
    // subtraction, with the corners falling out of the same field as the edges
    // rather than having to be lined up with them.
    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let thickness = 6.0;
    let mut whole = field_layer(0.0, 0.0, 64.0, Shape::Box);
    whole.operation = Operation::Union;
    let mut hole = field_layer(thickness, thickness, 64.0 - thickness * 2.0, Shape::Box);
    hole.operation = Operation::Subtract;
    hole.radii = [10.0; 4];
    let list = DrawList {
        commands: vec![field_command(node, vec![whole, hole])],
        layers: Vec::new(),
    };

    let pixels = render_readback(&list, 64);

    assert_eq!(alpha_at(&pixels, 64, 32, 1), 255, "the top edge is drawn");
    assert_eq!(alpha_at(&pixels, 64, 1, 32), 255, "and the left edge");
    assert_eq!(alpha_at(&pixels, 64, 62, 32), 255, "and the right edge");
    assert_eq!(alpha_at(&pixels, 64, 32, 62), 255, "and the bottom edge");
    // The middle is the hole: a frame that fills its own interior is a filled
    // rectangle covering the whole screen, which is exactly the failure worth
    // catching here.
    assert_eq!(alpha_at(&pixels, 64, 32, 32), 0, "the middle is open");
    // And the corner is inside the border, because the radius rounds the hole
    // rather than the outside.
    assert_eq!(alpha_at(&pixels, 64, 3, 3), 255, "the corner is filled");
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn an_oversized_corner_radius_makes_a_capsule_through_every_path() {
    // `radius = 9999` on a wide short box is how a configuration asks for a
    // pill. It only means that if the radius is clamped to the box's own half
    // extent — and it used to be clamped in the field shader and in the input
    // region rasteriser but not in the one that draws a Rect, so the same
    // numbers painted a capsule in two places and a plain square in the third.
    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut layer = field_layer(8.0, 20.0, 48.0, Shape::Box);
    layer.bounds.height = 24.0;
    layer.radii = [9_999.0; 4];
    let list = DrawList {
        commands: vec![field_command(node, vec![layer])],
        layers: Vec::new(),
    };

    let pixels = render_readback(&list, 64);

    // The middle of the pill is solid and its flat sides reach the ends.
    assert_eq!(alpha_at(&pixels, 64, 32, 32), 255, "the body is filled");
    assert_eq!(
        alpha_at(&pixels, 64, 12, 32),
        255,
        "and the rounded left end"
    );
    assert_eq!(
        alpha_at(&pixels, 64, 52, 32),
        255,
        "and the rounded right end"
    );
    // A square would keep its corners. A capsule cannot: the corner of the
    // layer box is a full radius away from any ink.
    assert_eq!(
        alpha_at(&pixels, 64, 9, 22),
        0,
        "the top-left corner is cut"
    );
    assert_eq!(
        alpha_at(&pixels, 64, 54, 42),
        0,
        "and the bottom-right corner"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_rotated_layer_is_not_sliced_flat_by_its_own_quad() {
    // The shader rotates the sample point into each layer's frame, so a rotated
    // layer covers a different rectangle than the one it was given. The quad is
    // built from those rectangles, and taking them unrotated meant a long thin
    // shape turned 45° had its ends cut off square by the very quad meant to
    // contain it.
    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let mut layer = field_layer(14.0, 26.0, 36.0, Shape::Box);
    layer.bounds.height = 12.0;
    layer.rotation = 45.0;
    let list = DrawList {
        commands: vec![field_command(node, vec![layer])],
        layers: Vec::new(),
    };

    let pixels = render_readback(&list, 64);

    assert_eq!(alpha_at(&pixels, 64, 32, 32), 255, "the middle is drawn");
    // Turned 45°, the bar runs corner to corner, and its ends land well outside
    // the unrotated 36x12 box — which is exactly where the old quad stopped.
    assert_eq!(alpha_at(&pixels, 64, 22, 22), 255, "and one end");
    assert_eq!(alpha_at(&pixels, 64, 42, 42), 255, "and the other");
    // The perpendicular diagonal is off the bar entirely.
    assert_eq!(alpha_at(&pixels, 64, 20, 44), 0, "not across the short way");
}

#[test]
#[ignore = "requires a GPU adapter"]
pub(crate) fn a_reused_layer_target_does_not_carry_the_last_frame_into_this_one() {
    // Offscreen layer targets are pooled and reused rather than allocated per
    // frame — a full-screen GPU texture per layer, sixty times a second, was
    // being created and thrown away. Reuse is only safe because every layer
    // pass clears before it draws, and that is what this checks: the same scene
    // drawn twice through one backend has to come out the same both times, and
    // a scene drawn after a busier one must not keep any of it.
    let mut scene = morf_scene::Scene::new();
    let node = scene.create(morf_scene::Element::Sdf);
    let framed = |x: f64| {
        let mut layer = field_layer(x, 20.0, 24.0, Shape::Circle);
        layer.operation = Operation::Union;
        DrawList {
            commands: vec![field_command(node, vec![layer])],
            layers: Vec::new(),
        }
    };

    let mut backend = pollster::block_on(WgpuBackend::new(64, 64)).unwrap();
    let busy = read_frame(&mut backend, &framed(4.0), 64);
    let first = read_frame(&mut backend, &framed(30.0), 64);
    let again = read_frame(&mut backend, &framed(30.0), 64);

    assert_eq!(first, again, "the same scene renders the same twice");
    assert_ne!(busy, first, "and a different scene really did differ");
    // Where the busy frame had ink and this one does not, nothing is left over.
    assert_eq!(alpha_at(&first, 64, 10, 32), 0, "the old circle is gone");
}
