use super::*;
use mold_scene::{Element, Scene};

#[test]
fn tessellates_svg_curves_and_caches_scale() {
    let mut cache = PathCache::default();
    let path = "M2 14 C2 4 14 4 14 14 Z";
    let first = cache.tessellate(path, 2.0, false, 120).unwrap();
    assert!(!first.fill.indices.is_empty());
    assert!(!first.stroke.indices.is_empty());
    let first_vertices = first.fill.vertices.len();
    let second = cache.tessellate(path, 2.0, false, 120).unwrap();
    assert_eq!(second.fill.vertices.len(), first_vertices);
}

#[test]
fn a_tessellated_fill_is_skirted_by_an_outward_coverage_band() {
    let mut cache = PathCache::default();
    // A unit-scale triangle, so the band width is one path unit.
    let mesh = cache
        .tessellate("M0 0 L10 0 L10 10 Z", 0.0, false, 120)
        .unwrap()
        .clone();

    let solid = mesh
        .fill
        .vertices
        .iter()
        .filter(|vertex| vertex[2] < -1.0)
        .count();
    let lip = mesh
        .fill
        .vertices
        .iter()
        .filter(|vertex| vertex[2] == 1.0)
        .count();
    assert!(solid >= 3, "the filled interior survived the band pass");
    assert!(lip >= 3, "every outline vertex grew an outer lip");
    assert_eq!(
        lip,
        mesh.fill
            .vertices
            .iter()
            .filter(|vertex| vertex[2] == 0.0)
            .count(),
        "the band is a closed ring, so its two rims match"
    );

    // The lip sits outside the shape it smooths, which is what keeps it
    // from blending a second time over the fill underneath.
    let corners = [(0.0_f32, 0.0_f32), (10.0, 0.0), (10.0, 10.0)];
    let inside = |x: f32, y: f32| {
        let side = |from: (f32, f32), to: (f32, f32)| {
            (x - from.0) * (to.1 - from.1) - (y - from.1) * (to.0 - from.0)
        };
        let sides = [
            side(corners[0], corners[1]),
            side(corners[1], corners[2]),
            side(corners[2], corners[0]),
        ];
        !(sides.iter().any(|value| *value < -1e-3) && sides.iter().any(|value| *value > 1e-3))
    };
    assert!(
        mesh.fill
            .vertices
            .iter()
            .filter(|vertex| vertex[2] == 1.0)
            .all(|vertex| !inside(vertex[0], vertex[1])),
        "an outer rim vertex fell back inside the triangle"
    );
}

#[test]
fn the_coverage_band_narrows_as_the_render_scale_grows() {
    let mut cache = PathCache::default();
    let mut extent = |scale_120: u32| {
        let mesh = cache
            .tessellate("M0 0 L10 0 L10 10 Z", 0.0, false, scale_120)
            .unwrap();
        mesh.fill
            .vertices
            .iter()
            .filter(|vertex| vertex[2] == 1.0)
            .map(|vertex| vertex[0])
            .fold(f32::MIN, f32::max)
    };
    // The band is a pixel wide on screen, so in path units it has to shrink
    // as the geometry is tessellated for a denser target.
    assert!(extent(240) < extent(120));
}

#[test]
fn parametric_shape_names_are_accepted_and_differ_by_parameter() {
    assert!(is_morph_shape("star:9:0.4"));
    assert!(is_morph_shape("polygon:6:0.25"));
    assert!(is_morph_shape("circle:24"));
    // A family with no parameters still resolves through its defaults.
    assert!(is_morph_shape("polygon"));
    // Anything that is not a known family, or carries a non-numeric
    // parameter, stays unknown rather than silently becoming a default.
    assert!(!is_morph_shape("spiral:4"));
    assert!(!is_morph_shape("star:many"));
    assert!(morph_shape("star:many").is_err());

    // Counted through a self-morph, since the cubic count is what topology
    // matching later has to reconcile between two different shapes.
    let outline = |name: &str| {
        Morph::new(morph_shape(name).unwrap(), morph_shape(name).unwrap())
            .as_cubics(0.0)
            .len()
    };
    assert_ne!(
        outline("star:5:0.4"),
        outline("star:9:0.4"),
        "a different point count must produce a different outline"
    );
}

#[test]
fn a_parametric_shape_morphs_against_a_built_in_one() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Shape);
    let bounds = Geometry {
        x: 0.0,
        y: 0.0,
        width: 120.0,
        height: 120.0,
    };
    let mut cache = PathCache::default();
    let mut spec = ShapeMorph {
        from: "square".to_owned(),
        to: "star:7:0.55:0.2".to_owned(),
        progress: 0.0,
    };
    let start = cache
        .tessellate_morph(node, &spec, bounds, 0.0, false, 120)
        .unwrap()
        .clone();
    spec.progress = 1.0;
    let end = cache
        .tessellate_morph(node, &spec, bounds, 0.0, false, 120)
        .unwrap()
        .clone();

    assert!(!start.fill.indices.is_empty());
    assert_ne!(start.fill.vertices, end.fill.vertices);
}

#[test]
fn polymorpher_replaces_one_cached_mesh_per_scene_node() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Shape);
    let bounds = Geometry {
        x: 0.0,
        y: 0.0,
        width: 120.0,
        height: 120.0,
    };
    let mut cache = PathCache::default();
    let mut spec = ShapeMorph {
        from: "square".to_owned(),
        to: "circle".to_owned(),
        progress: 0.0,
    };
    let square = cache
        .tessellate_morph(node, &spec, bounds, 0.0, false, 120)
        .unwrap()
        .clone();
    spec.progress = 1.0;
    let circle = cache
        .tessellate_morph(node, &spec, bounds, 0.0, false, 120)
        .unwrap()
        .clone();

    assert_ne!(square.fill.vertices, circle.fill.vertices);
    assert_eq!(cache.morphs.len(), 1);
    assert_eq!(cache.morphed.len(), 1);
}
