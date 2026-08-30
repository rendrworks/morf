// Built-in and parametric Polymorpher shapes named by `morph_from`/`morph_to`.

/// Shape families that take colon-separated parameters after their name.
///
/// A parametric name is what lets a shell reach shapes the built-in table does
/// not enumerate — a nine-pointed star, a hexagon with a specific corner
/// radius — without mold shipping a name for every one of them.
const PARAMETRIC_SHAPES: [&str; 6] = [
    "polygon",
    "star",
    "circle",
    "rectangle",
    "pill",
    "pill_star",
];

/// Splits a shape name into its family and its numeric parameters.
fn shape_parameters(name: &str) -> Option<(&str, Vec<f32>)> {
    let (family, rest) = name.split_once(':')?;
    if !PARAMETRIC_SHAPES.contains(&family) {
        return None;
    }
    let parameters = rest
        .split(':')
        .map(|part| {
            part.trim()
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
        })
        .collect::<Option<Vec<_>>>()?;
    (!parameters.is_empty()).then_some((family, parameters))
}

pub(crate) fn is_morph_shape(name: &str) -> bool {
    if shape_parameters(name).is_some() {
        return true;
    }
    matches!(
        name,
        "circle"
            | "square"
            | "slanted"
            | "arch"
            | "fan"
            | "arrow"
            | "semi_circle"
            | "oval"
            | "pill"
            | "triangle"
            | "diamond"
            | "clam_shell"
            | "pentagon"
            | "gem"
            | "sunny"
            | "very_sunny"
            | "cookie4"
            | "cookie6"
            | "cookie7"
            | "cookie9"
            | "cookie12"
            | "ghostish"
            | "clover4"
            | "clover8"
            | "burst"
            | "soft_burst"
            | "boom"
            | "soft_boom"
            | "flower"
            | "puffy"
            | "puffy_diamond"
            | "pixel_circle"
            | "pixel_triangle"
            | "bun"
            | "heart"
            | "polygon"
            | "rectangle"
            | "pill_star"
    )
}

/// Reads a parameter by position, falling back to the family's own default.
fn parameter(parameters: &[f32], index: usize, fallback: f32) -> f32 {
    parameters.get(index).copied().unwrap_or(fallback)
}

/// Reads a positive vertex count, which every family needs at least three of.
fn vertex_count(parameters: &[f32], index: usize, fallback: usize) -> usize {
    parameters
        .get(index)
        .map(|value| (*value as usize).max(3))
        .unwrap_or(fallback)
}

/// Builds a shape family from its numeric parameters.
///
/// Every result is normalized into the unit box the morph path sampler expects,
/// so a parametric shape composes with a built-in one in the same morph.
fn parametric_shape(family: &str, parameters: &[f32]) -> Result<RoundedPolygon, String> {
    let shape = match family {
        // A regular n-gon: vertex count, then corner radius and smoothing.
        "polygon" => RoundedPolygon::from_vertices_count(
            vertex_count(parameters, 0, 6),
            1.0,
            Some(CornerRounding::smoothed(
                parameter(parameters, 1, 0.0).clamp(0.0, 1.0),
                parameter(parameters, 2, 0.0).clamp(0.0, 1.0),
            )),
            &[],
        ),
        // Point count, inner radius as a fraction of the outer one, then the
        // corner radius applied to the outer and inner vertices in turn.
        "star" => {
            let points = vertex_count(parameters, 0, 5);
            let inner = parameter(parameters, 1, 0.5).clamp(0.01, 1.0);
            RoundedPolygon::star(points)
                .with_inner_radius(inner)
                .with_rounding(CornerRounding::new(
                    parameter(parameters, 2, 0.0).clamp(0.0, 1.0),
                ))
                .with_inner_rounding(CornerRounding::new(
                    parameter(parameters, 3, parameter(parameters, 2, 0.0)).clamp(0.0, 1.0),
                ))
                .build()
        }
        // Segment count. More segments track a true circle more closely and
        // give a morph against a many-cornered shape more to match against.
        "circle" => RoundedPolygon::circle()
            .with_vertices(vertex_count(parameters, 0, 10))
            .build(),
        // Corner radius and smoothing, on a square by default.
        "rectangle" => RoundedPolygon::rectangle()
            .with_rounding(CornerRounding::smoothed(
                parameter(parameters, 0, 0.0).clamp(0.0, 1.0),
                parameter(parameters, 1, 0.0).clamp(0.0, 1.0),
            ))
            .build(),
        // Endcap smoothing, then the width and height the pill is drawn at.
        "pill" => RoundedPolygon::pill()
            .with_smoothing(parameter(parameters, 0, 0.0).clamp(0.0, 1.0))
            .with_size(Size::new(
                parameter(parameters, 1, 2.0).max(0.01),
                parameter(parameters, 2, 1.0).max(0.01),
            ))
            .build(),
        // Point count and inner radius ratio, then how vertices are spaced
        // around the rounded ends.
        "pill_star" => RoundedPolygon::pill_star()
            .with_vertices_per_radius(vertex_count(parameters, 0, 8))
            .with_inner_radius_ratio(parameter(parameters, 1, 0.5).clamp(0.01, 1.0))
            .with_vertex_spacing(parameter(parameters, 2, 0.5).clamp(0.0, 1.0))
            .build(),
        _ => return Err(format!("unknown Polymorpher shape family `{family}`")),
    };
    Ok(shape.normalized())
}

fn morph_shape(name: &str) -> Result<RoundedPolygon, String> {
    if let Some((family, parameters)) = shape_parameters(name) {
        return parametric_shape(family, &parameters);
    }
    let shape = match name {
        "polygon" | "rectangle" | "pill_star" => return parametric_shape(name, &[]),
        "circle" => shapes::circle(None),
        "square" => shapes::square(),
        "slanted" => shapes::slanted(),
        "arch" => shapes::arch(),
        "fan" => shapes::fan(),
        "arrow" => shapes::arrow(),
        "semi_circle" => shapes::semi_circle(),
        "oval" => shapes::oval(),
        "pill" => shapes::pill(),
        "triangle" => shapes::triangle(),
        "diamond" => shapes::diamond(),
        "clam_shell" => shapes::clam_shell(),
        "pentagon" => shapes::pentagon(),
        "gem" => shapes::gem(),
        "sunny" => shapes::sunny(),
        "very_sunny" => shapes::very_sunny(),
        "cookie4" => shapes::cookie4(),
        "cookie6" => shapes::cookie6(),
        "cookie7" => shapes::cookie7(),
        "cookie9" => shapes::cookie9(),
        "cookie12" => shapes::cookie12(),
        "ghostish" => shapes::ghostish(),
        "clover4" => shapes::clover4(),
        "clover8" => shapes::clover8(),
        "burst" => shapes::burst(),
        "soft_burst" => shapes::soft_burst(),
        "boom" => shapes::boom(),
        "soft_boom" => shapes::soft_boom(),
        "flower" => shapes::flower(),
        "puffy" => shapes::puffy(),
        "puffy_diamond" => shapes::puffy_diamond(),
        "pixel_circle" => shapes::pixel_circle(),
        "pixel_triangle" => shapes::pixel_triangle(),
        "bun" => shapes::bun(),
        "heart" => shapes::heart(),
        _ => return Err(format!("unknown Polymorpher shape `{name}`")),
    };
    Ok(shape)
}
