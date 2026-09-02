use super::fields::field_scene;
use super::*;

#[test]
fn layers_are_packed_into_the_fields_own_space_and_scaled() {
    // The shader walks the field in its own coordinates, so a layer's centre is
    // relative to the node's top-left corner and not to the surface. Every
    // length is scaled to physical pixels; the ratios and the counts are not.
    let (scene, root, _) = field_scene(&[(
        Element::SdfShape,
        &[
            ("x", Value::Number(20.0)),
            ("y", Value::Number(30.0)),
            ("width", Value::Number(80.0)),
            ("height", Value::Number(60.0)),
            ("shape", Value::String("ring".into())),
            ("morph_to", Value::String("hexagon".into())),
            ("morph_progress", Value::Number(0.25)),
            ("thickness", Value::Number(4.0)),
            ("points", Value::Number(7.0)),
            ("inner_radius", Value::Number(0.4)),
            ("rotation", Value::Number(30.0)),
        ],
    )]);
    // Place the field away from the origin so the two spaces cannot coincide.
    let mut scene = scene;
    scene.assign(root, "x", 500.0).unwrap();
    scene.assign(root, "y", 400.0).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 200.0,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let command = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap();

    let mut layers = Vec::new();
    let mut materials = Vec::new();
    // 240/120 is a doubled surface: every length doubles.
    let instance = SdfFieldInstance::from_command(
        command,
        240,
        &mut layers,
        &mut materials,
        &mut Vec::new(),
        &mut morf_text::TextSystem::new(),
        &mut morf_svg::SvgOutlines::new(),
    )
    .unwrap();

    assert_eq!(layers.len(), 1);
    // Centre is (20 + 40, 30 + 30) inside the field, doubled.
    assert_eq!(layers[0].rect, [120.0, 120.0, 80.0, 60.0]);
    assert_eq!(layers[0].kinds[0], Shape::Ring.code() as f32);
    assert_eq!(layers[0].kinds[1], Shape::Hexagon.code() as f32);
    assert_eq!(layers[0].kinds[2], 0.25);
    assert_eq!(layers[0].params[1], 7.0, "a point count is not a length");
    assert_eq!(layers[0].params[2], 0.4, "a ratio is not a length");
    assert_eq!(layers[0].params[3], 8.0, "thickness is a length");
    assert_eq!(layers[0].extra[1], 30.0, "degrees are not a length");
    assert_eq!(instance.style[2], 0.0, "first layer");
    assert_eq!(instance.style[3], 1.0, "layer count");
}

#[test]
fn layer_runs_are_addressed_per_field_within_one_shared_buffer() {
    // Two fields in one frame write into the same buffer, so the second has to
    // start where the first ended rather than at zero.
    let (scene, _, layout) = field_scene(&[
        (
            Element::SdfShape,
            &[
                ("width", Value::Number(10.0)),
                ("height", Value::Number(10.0)),
            ],
        ),
        (
            Element::SdfShape,
            &[
                ("width", Value::Number(10.0)),
                ("height", Value::Number(10.0)),
            ],
        ),
    ]);
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let command = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap();

    let mut layers = Vec::new();
    let mut materials = Vec::new();
    let first = SdfFieldInstance::from_command(
        command,
        120,
        &mut layers,
        &mut materials,
        &mut Vec::new(),
        &mut morf_text::TextSystem::new(),
        &mut morf_svg::SvgOutlines::new(),
    )
    .unwrap();
    let second = SdfFieldInstance::from_command(
        command,
        120,
        &mut layers,
        &mut materials,
        &mut Vec::new(),
        &mut morf_text::TextSystem::new(),
        &mut morf_svg::SvgOutlines::new(),
    )
    .unwrap();

    assert_eq!(first.style[2], 0.0);
    assert_eq!(first.style[3], 2.0);
    assert_eq!(second.style[2], 2.0);
    assert_eq!(second.style[3], 2.0);
    assert_eq!(layers.len(), 4);
}

#[test]
fn a_composition_past_the_cap_is_truncated_rather_than_unbounded() {
    // Every layer costs every pixel of the node, so the count the shader walks
    // is bounded whatever the configuration asks for.
    let many: Vec<_> = (0..MAX_FIELD_LAYERS + 5)
        .map(|_| {
            (
                Element::SdfShape,
                &[
                    ("width", Value::Number(10.0)),
                    ("height", Value::Number(10.0)),
                ][..],
            )
        })
        .collect();
    let (scene, _, layout) = field_scene(&many);
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let command = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap();

    let mut layers = Vec::new();
    let mut materials = Vec::new();
    let instance = SdfFieldInstance::from_command(
        command,
        120,
        &mut layers,
        &mut materials,
        &mut Vec::new(),
        &mut morf_text::TextSystem::new(),
        &mut morf_svg::SvgOutlines::new(),
    )
    .unwrap();

    assert_eq!(layers.len(), MAX_FIELD_LAYERS);
    assert_eq!(instance.style[3], MAX_FIELD_LAYERS as f32);
}

#[test]
fn field_bounds_cover_the_layers_the_outline_and_the_softened_edge() {
    // A layer may reach outside the node, and the outline and softness spread
    // outwards from every crossing. Damage that missed them would leave a
    // smear on screen wherever the field moved.
    let (scene, root, _) = field_scene(&[(
        Element::SdfShape,
        &[
            ("x", Value::Number(-30.0)),
            ("y", Value::Number(10.0)),
            ("width", Value::Number(50.0)),
            ("height", Value::Number(50.0)),
        ],
    )]);
    let mut scene = scene;
    scene.assign(root, "stroke_width", 4.0).unwrap();
    scene.assign(root, "softness", 3.0).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 200.0,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let command = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap();

    // The area is the layer, not the node it sits in — a field paints only
    // where a layer reaches, so the empty part of a large node is neither drawn
    // nor damaged. The layer runs from -30 to 20 across and 10 to 60 down, and
    // the outline and softened edge add 4/2 + 3 on every side.
    let bounds = command.bounds();
    assert_eq!(bounds.x, -35.0);
    assert_eq!(bounds.y, 5.0);
    assert_eq!(bounds.width, 60.0);
    assert_eq!(bounds.height, 60.0);
}

#[test]
fn a_blend_widens_the_area_a_field_may_reach() {
    // A smooth seam pushes the surface *outward* where two shapes meet, so the
    // area the field is drawn into has to allow for it. Sized without the
    // blend, the bulge is clipped flat and a fused row of cards comes out with
    // the top and bottom of every join sliced off.
    let layers = |blend: f32| {
        vec![SdfLayer {
            glyph: None,
            glyph_morph_to: None,
            svg_source: None,
            svg_source_morph_to: None,
            font_family: None,
            font_family_morph_to: None,
            bounds: Geometry {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
            },
            color: Color::rgba8(255, 255, 255, 255),
            shape: Shape::Circle,
            morph_to: Shape::Circle,
            morph: 0.0,
            operation: Operation::SmoothUnion,
            blend,
            rotation: 0.0,
            radii: [0.0; 4],
            points: 5.0,
            inner_radius: 0.5,
            thickness: 0.0,
            angle: 90.0,
        }]
    };

    assert_eq!(field_spread(0.0, 0.0, &layers(0.0)), 0.0);
    assert_eq!(field_spread(0.0, 0.0, &layers(18.0)), 18.0);
    // The outline and the softened edge are on top of it, not instead of it.
    assert_eq!(field_spread(4.0, 3.0, &layers(18.0)), 23.0);
}

/// The field shader walks a polygon layer in runs of a fixed length rather than
/// being told one, because every contour is resampled to the same size when the
/// outline is built. That length is written into `field.wgsl`, so a change here
/// has to be a change there — this is what says so.
#[test]
fn the_shader_and_the_outline_agree_on_a_contour_length() {
    assert_eq!(morf_text::GLYPH_CONTOUR_POINTS, 96);
    let shader = include_str!("../field.wgsl");
    assert!(
        shader.contains("sd_polygon(point, u32(layer.params.x), 96u, u32(layer.extra.w))"),
        "field.wgsl must walk polygon contours in runs of GLYPH_CONTOUR_POINTS"
    );
}

/// A field finds its layers by an index carried from the vertex stage, and an
/// index cannot survive being interpolated.
///
/// `style.z` and `style.w` are the first layer's index and the layer count.
/// They are the same number at all four corners of the quad, but a varying
/// that is not `flat` is interpolated anyway, and a 7.0 written at every corner
/// comes back as 6.9999997 in the middle. `u32()` truncates towards zero, so
/// the field read the *previous* field's layers and drew nothing anyone asked
/// for. Which fields it hit depended on their layer index, so shapes went
/// missing at what looked like random — and it survived every CPU test in this
/// file, because every number on this side of the buffer was correct.
#[test]
fn the_field_shader_does_not_interpolate_what_it_indexes_with() {
    let shader = include_str!("../field.wgsl");
    for varying in [
        "@location(1) @interpolate(flat) fill",
        "@location(2) @interpolate(flat) outline",
        "@location(3) @interpolate(flat) style",
        "@location(4) @interpolate(flat) material",
    ] {
        assert!(
            shader.contains(varying),
            "field.wgsl must carry `{varying}` flat: it is one value per instance"
        );
    }
}

/// The boxes that let a fragment skip most of a contour are packed behind the
/// points by the renderer and found by arithmetic in the shader, so the two
/// have to agree on how many edges one box holds.
#[test]
fn the_shader_and_the_outline_agree_on_a_run_length() {
    let shader = include_str!("../field.wgsl");
    assert!(
        shader.contains(&format!(
            "const OUTLINE_SPAN: u32 = {}u;",
            crate::field::glyph_layer::OUTLINE_SPAN
        )),
        "field.wgsl must box outline edges in runs of OUTLINE_SPAN"
    );
}
