use super::field_tests::{field_command, field_layer, render_readback};
use super::*;
use crate::{SdfOperation, SdfShapeKind};

#[test]
#[ignore = "requires a GPU adapter"]
fn a_fused_surface_carries_each_layer_s_own_colour_across_the_seam() {
    // A composition is one surface but not one colour. Two blobs joined by a
    // smooth union keep their own fills and cross-fade exactly where they bulge
    // into each other, so a fused row of differently coloured shapes stays
    // legible instead of flattening to a single fill.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let mut left = field_layer(4.0, 20.0, 26.0, SdfShapeKind::Circle);
    left.color = Color::rgba8(255, 0, 0, 255);
    let mut right = field_layer(34.0, 20.0, 26.0, SdfShapeKind::Circle);
    right.color = Color::rgba8(0, 0, 255, 255);
    right.operation = SdfOperation::SmoothUnion;
    right.blend = 22.0;
    let pixels = render_readback(
        &DrawList {
            commands: vec![field_command(node, vec![left, right])],
            layers: Vec::new(),
        },
        64,
    );
    let at = |x: u32, y: u32| {
        let i = ((y * 64 + x) * 4) as usize;
        (pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3])
    };

    let (lr, _, lb, la) = at(12, 33);
    let (rr, _, rb, ra) = at(52, 33);
    assert_eq!(la, 255, "left blob is painted");
    assert_eq!(ra, 255, "right blob is painted");
    assert!(lr > 200 && lb < 60, "left keeps its own red: {lr},{lb}");
    assert!(rb > 200 && rr < 60, "right keeps its own blue: {rr},{rb}");

    // The neck between them is a mixture of the two, not either end.
    let (nr, _, nb, na) = at(32, 33);
    assert_eq!(na, 255, "the neck is filled");
    assert!(
        nr > 20 && nb > 20,
        "the seam should carry both fills, got r={nr} b={nb}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_box_keeps_a_radius_per_corner() {
    // A field absorbs ordinary rects, and a rect carries four corner radii. If
    // the box collapsed them to one, a card rounded only along its top would
    // come out rounded all round once it joined a composition.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let mut layer = field_layer(8.0, 8.0, 48.0, SdfShapeKind::Box);
    // Top-left, top-right, bottom-right, bottom-left.
    layer.radii = [22.0, 0.0, 22.0, 0.0];
    let pixels = render_readback(
        &DrawList {
            commands: vec![field_command(node, vec![layer])],
            layers: Vec::new(),
        },
        64,
    );
    let alpha = |x: u32, y: u32| pixels[((y * 64 + x) * 4 + 3) as usize];

    // The two rounded corners are cut away; the two square ones are filled.
    assert_eq!(alpha(10, 10), 0, "top-left is rounded");
    assert_eq!(alpha(53, 53), 0, "bottom-right is rounded");
    assert_eq!(alpha(53, 10), 255, "top-right is square");
    assert_eq!(alpha(10, 53), 255, "bottom-left is square");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_smooth_seam_may_bulge_outside_the_node_without_being_clipped() {
    // A smooth operator returns a value below either input, so the surface is
    // larger than the shapes wherever the seam is active — including outside
    // the node's own rectangle. The quad has to allow for that; sized only for
    // the outline and the softened edge it slices the bulge flat, which is what
    // a fused row of cards looks like with the top and bottom of every join
    // cut off.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    // A node in the middle of the target, with a box filling it exactly, so
    // anything the seam adds necessarily lands outside the node.
    let field = |blend: f32| {
        let mut filled = field_layer(18.0, 18.0, 28.0, SdfShapeKind::Box);
        filled.radii = [4.0; 4];
        let mut joined = field_layer(30.0, 18.0, 28.0, SdfShapeKind::Circle);
        joined.operation = SdfOperation::SmoothUnion;
        joined.blend = blend;
        DrawCommand::Field {
            node,
            bounds: Geometry {
                x: 18.0,
                y: 18.0,
                width: 28.0,
                height: 28.0,
            },
            transform: Transform2D::IDENTITY,
            clip: None,
            fill_color: Color::rgba8(255, 255, 255, 255),
            stroke_color: Color::rgba8(0, 0, 0, 0),
            stroke_width: 0.0,
            softness: 0.0,
            layers: vec![filled, joined],
        }
    };
    let covered = |blend: f32| {
        let pixels = render_readback(
            &DrawList {
                commands: vec![field(blend)],
                layers: Vec::new(),
            },
            64,
        );
        (0..64 * 64)
            .filter(|index| pixels[index * 4 + 3] > 128)
            .count()
    };

    let hard = covered(0.0);
    let fused = covered(14.0);

    // The seam adds surface, and none of it may be lost to the quad.
    assert!(
        fused > hard,
        "a blend should add surface, got {fused} against {hard}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn alpha_cross_fades_across_a_seam_like_any_other_channel() {
    // Transparency is per layer, and it is carried through the seam rather than
    // resolved by drawing one shape over another. Where an opaque shape fuses
    // into a transparent one the *surface* becomes gradually see-through; there
    // is no edge where one sits on top of the other.
    let mut scene = mold_scene::Scene::new();
    let node = scene.create(mold_scene::Element::Sdf);
    let mut solid = field_layer(4.0, 20.0, 26.0, SdfShapeKind::Circle);
    solid.color = Color::rgba8(255, 255, 255, 255);
    let mut faint = field_layer(34.0, 20.0, 26.0, SdfShapeKind::Circle);
    faint.color = Color::rgba8(255, 255, 255, 40);
    faint.operation = SdfOperation::SmoothUnion;
    faint.blend = 22.0;
    let pixels = render_readback(
        &DrawList {
            commands: vec![field_command(node, vec![solid, faint])],
            layers: Vec::new(),
        },
        64,
    );
    let alpha = |x: u32, y: u32| pixels[((y * 64 + x) * 4 + 3) as usize];

    let opaque_end = alpha(12, 33);
    let faint_end = alpha(52, 33);
    let seam = alpha(32, 33);

    assert!(opaque_end > 240, "the solid end stays solid: {opaque_end}");
    assert!(faint_end < 90, "the faint end stays faint: {faint_end}");
    // And the join is neither end: the surface fades across it.
    assert!(
        seam > faint_end && seam < opaque_end,
        "the seam should fade between {faint_end} and {opaque_end}, got {seam}"
    );
}
