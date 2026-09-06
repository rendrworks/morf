//! The one shape vocabulary: what morf can draw, and what it can make clickable.
//!
//! There used to be two. The renderer knew nine analytic families and six ways
//! to combine them; the input region knew a rectangle, an ellipse, and four
//! boolean operations. A configuration could therefore draw a star and then
//! discover that the only clickable area it could give that star was a
//! rectangle — the same object described twice, in two vocabularies, with
//! different words in the config API for each.
//!
//! This module is the merge. Every family below has both an exact analytic
//! distance function here on the CPU and a twin in `field.wgsl`, and the two
//! are held together by the agreement tests: for a shape to exist at all, it
//! must be drawable *and* clickable.
//!
//! The distance functions are ports of the shader's, arithmetic for
//! arithmetic, and the discriminants are the ones the shader switches on — so
//! the order of these variants is load-bearing and new families append.

/// A family of shape.
///
/// The discriminant is what `field.wgsl`'s `shape_distance` switches on, so
/// these may be appended to but never reordered.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Shape {
    /// Inscribed circle: the radius is the box's shorter half-extent, so it
    /// stays a circle in a rectangle that is not square. Use `Ellipse` to fill
    /// the box instead.
    #[default]
    Circle,
    /// Rectangle with a radius per corner.
    Box,
    /// Stadium: a rectangle with fully rounded ends.
    Capsule,
    Triangle,
    Hexagon,
    /// `points`-pointed star, waisted by `inner_radius`.
    Star,
    /// Annulus of the given `thickness`.
    Ring,
    /// Circular sector spanning `angle` degrees, centred on straight up.
    Pie,
    /// Plus sign with arms `thickness` wide.
    Cross,
    /// Ellipse stretched to fill the box, which is what an input region has
    /// always meant by "ellipse" and what the renderer had no way to draw.
    Ellipse,
    /// A closed outline given as points rather than described by a formula.
    ///
    /// This is how a letter joins a field. A glyph is not a family of shape
    /// with parameters — it is a particular outline — so the only way for one
    /// to union, subtract or morph with a circle is for the composition to
    /// accept an outline as a shape in its own right. The points live in a
    /// buffer beside the layers; the layer says where its own run begins.
    Polygon,
}

impl Shape {
    /// Parses the name a configuration uses.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "circle" => Self::Circle,
            "box" | "rect" | "rectangle" => Self::Box,
            "capsule" | "pill" | "stadium" => Self::Capsule,
            "triangle" => Self::Triangle,
            "hexagon" => Self::Hexagon,
            "star" => Self::Star,
            "ring" | "annulus" => Self::Ring,
            "pie" | "sector" => Self::Pie,
            "cross" | "plus" => Self::Cross,
            "ellipse" | "oval" => Self::Ellipse,
            // Nameable so a layer can morph *into* an outline as well as out of
            // one: `shape = "star", morph_to = "glyph"` is a star becoming a
            // letter, which is the same interpolation as a star becoming a
            // hexagon. The points come from the layer's `glyph`.
            // And `svg`, which is the same thing again: a drawing is an outline,
            // a letter is an outline, and the field walks whichever it is
            // handed. Which one a layer means is said by naming a `glyph` or a
            // `source`, not by naming a different shape.
            "glyph" | "polygon" | "svg" | "outline" => Self::Polygon,
            _ => return None,
        })
    }

    /// The name a configuration reads back.
    pub fn name(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::Box => "rect",
            Self::Capsule => "capsule",
            Self::Triangle => "triangle",
            Self::Hexagon => "hexagon",
            Self::Polygon => "glyph",
            Self::Star => "star",
            Self::Ring => "ring",
            Self::Pie => "pie",
            Self::Cross => "cross",
            Self::Ellipse => "ellipse",
        }
    }

    /// The discriminant the shader switches on.
    pub fn code(self) -> u32 {
        self as u32
    }

    /// Whether the family covers its whole rectangle, corners included.
    ///
    /// The region rasteriser fills those a row at a time instead of testing
    /// every pixel, which is the difference between a bar costing its own area
    /// in integer writes and costing it in distance evaluations.
    pub fn fills_box(self, params: &ShapeParams) -> bool {
        matches!(self, Self::Box) && params.radii == [0.0; 4]
    }
}

/// How one shape joins the composition before it.
///
/// As with `Shape`, the discriminant is the shader's, so variants append.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Operation {
    #[default]
    Union,
    Subtract,
    Intersect,
    /// Union whose seam is rounded over `blend` pixels.
    SmoothUnion,
    /// Subtraction whose seam is rounded over `blend` pixels.
    SmoothSubtract,
    /// Intersection whose seam is rounded over `blend` pixels.
    SmoothIntersect,
    /// Symmetric difference: in one shape or the other, but not both.
    Xor,
}

impl Operation {
    /// Parses the name a configuration uses.
    ///
    /// `combine` is the input region's word for a union and stays accepted;
    /// both sides now answer to both names.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "union" | "combine" => Self::Union,
            "subtract" | "difference" => Self::Subtract,
            "intersect" | "intersection" => Self::Intersect,
            "smooth_union" => Self::SmoothUnion,
            "smooth_subtract" => Self::SmoothSubtract,
            "smooth_intersect" => Self::SmoothIntersect,
            "xor" => Self::Xor,
            _ => return None,
        })
    }

    /// The name a configuration reads back.
    pub fn name(self) -> &'static str {
        match self {
            Self::Union => "union",
            Self::Subtract => "subtract",
            Self::Intersect => "intersect",
            Self::SmoothUnion => "smooth_union",
            Self::SmoothSubtract => "smooth_subtract",
            Self::SmoothIntersect => "smooth_intersect",
            Self::Xor => "xor",
        }
    }

    /// The discriminant the shader switches on.
    pub fn code(self) -> u32 {
        self as u32
    }

    /// The hard operation a smooth one becomes when there is no seam to round.
    ///
    /// A boolean mask has no partial coverage, so the region rasteriser cannot
    /// represent a blended seam and composes the underlying set operation
    /// instead. The shapes agree; only the few pixels of the seam differ, and
    /// on a mask those pixels have to be in or out regardless.
    pub fn hard(self) -> Self {
        match self {
            Self::SmoothUnion => Self::Union,
            Self::SmoothSubtract => Self::Subtract,
            Self::SmoothIntersect => Self::Intersect,
            other => other,
        }
    }
}

/// The per-family parameters a shape needs beyond its rectangle.
///
/// Flat rather than carried inside the `Shape` variants, because this is
/// exactly what the GPU packs into a layer's uniform: keeping the two the same
/// shape is what makes the agreement tests a straight comparison.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeParams {
    /// Corner radii — top-left, top-right, bottom-right, bottom-left — for the
    /// families that have corners.
    pub radii: [f32; 4],
    /// Point count, for `Star`. Fractional counts are blended as fields.
    pub points: f32,
    /// Waist as a fraction of the outer radius, for `Star`.
    pub inner_radius: f32,
    /// Arm or wall thickness, for `Ring` and `Cross`.
    pub thickness: f32,
    /// Sector sweep in degrees, for `Pie`.
    pub angle: f32,
}

impl Default for ShapeParams {
    fn default() -> Self {
        Self {
            radii: [0.0; 4],
            points: 5.0,
            inner_radius: 0.5,
            thickness: 4.0,
            angle: 90.0,
        }
    }
}

impl ShapeParams {
    /// The parameters for a box with one radius on every corner.
    pub fn rounded(radius: f32) -> Self {
        Self {
            radii: [radius; 4],
            ..Self::default()
        }
    }
}

/// Signed distance from `point` to the shape, both measured from the centre of
/// a box whose half-extents are `half`. Negative is inside.
///
/// This is the CPU twin of `field.wgsl`'s `shape_distance`, and the agreement
/// tests exist to keep it that way. Where the shader is approximate — the
/// ellipse — the approximation is chosen so the *sign* stays exact, because
/// the sign is what decides whether a click lands.
pub fn distance(shape: Shape, params: &ShapeParams, half: [f32; 2], point: [f32; 2]) -> f32 {
    let radius = half[0].min(half[1]);
    match shape {
        Shape::Circle => sd_circle(point, radius),
        Shape::Box => sd_box(point, half, params.radii),
        Shape::Capsule => sd_box_uniform(point, half, half[0].min(half[1])),
        Shape::Triangle => sd_triangle(point, half),
        Shape::Hexagon => sd_hexagon(point, radius),
        Shape::Star => sd_star(point, radius, params.points, params.inner_radius),
        Shape::Ring => sd_ring(point, radius, params.thickness),
        Shape::Pie => sd_pie(point, radius, params.angle),
        Shape::Cross => sd_cross(point, half, params.thickness),
        Shape::Ellipse => sd_ellipse(point, half),
        // An input region has no points to walk — they live in the render
        // buffer, not here — so it takes the layer's box. A click test wants a
        // sign, and inside the box is the right answer for every point a
        // glyph-shaped hit area is asked about.
        Shape::Polygon => sd_box(point, half, [0.0; 4]),
    }
}

/// Combines a layer's distance into the composition so far.
///
/// `blend` is the seam radius in pixels, and is ignored by the hard operations.
pub fn combine(operation: Operation, accumulated: f32, layer: f32, blend: f32) -> f32 {
    let k = blend.max(0.0001);
    match operation {
        Operation::Union => accumulated.min(layer),
        Operation::Subtract => accumulated.max(-layer),
        Operation::Intersect => accumulated.max(layer),
        Operation::SmoothUnion => smooth_union(accumulated, layer, k),
        Operation::SmoothSubtract => -smooth_union(-accumulated, layer, k),
        Operation::SmoothIntersect => -smooth_union(-accumulated, -layer, k),
        // In one or the other but not both: outside their union, or inside
        // their intersection, whichever boundary is nearer.
        Operation::Xor => accumulated.min(layer).max(-accumulated.max(layer)),
    }
}

fn smooth_union(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b * (1.0 - h) + a * h - k * h * (1.0 - h)
}

fn sd_circle(point: [f32; 2], radius: f32) -> f32 {
    length(point) - radius
}

/// A box with a radius per corner.
///
/// The radius is clamped to the box's own half-extent, so asking for a corner
/// larger than the box can hold gives the largest one it can — a capsule, then
/// a circle — rather than a shape that inverts.
pub fn sd_box(point: [f32; 2], half: [f32; 2], radii: [f32; 4]) -> f32 {
    let r = if point[0] >= 0.0 {
        if point[1] >= 0.0 { radii[2] } else { radii[1] }
    } else if point[1] >= 0.0 {
        radii[3]
    } else {
        radii[0]
    };
    let r = r.max(0.0).min(half[0].min(half[1]));
    let q = [point[0].abs() - half[0] + r, point[1].abs() - half[1] + r];
    length([q[0].max(0.0), q[1].max(0.0)]) + q[0].max(q[1]).min(0.0) - r
}

fn sd_box_uniform(point: [f32; 2], half: [f32; 2], radius: f32) -> f32 {
    sd_box(point, half, [radius; 4])
}

fn sd_triangle(point: [f32; 2], half: [f32; 2]) -> f32 {
    // An equilateral triangle inscribed in the layer box, pointing up.
    let k = 3.0f32.sqrt();
    let r = half[0].min(half[1]);
    let mut p = [
        point[0] / half[0].max(0.0001) * r,
        -(point[1] / half[1].max(0.0001) * r),
    ];
    p[0] = p[0].abs() - r;
    p[1] += r / k;
    if p[0] + k * p[1] > 0.0 {
        p = [(p[0] - k * p[1]) / 2.0, (-k * p[0] - p[1]) / 2.0];
    }
    p[0] -= p[0].clamp(-2.0 * r, 0.0);
    -length(p) * p[1].signum()
}

/// A regular hexagon inscribed in the box.
///
/// The classic form takes the apothem, whose circumradius is `2r/sqrt(3)`, so
/// handing it the box's half-extent produced a hexagon wider than its own
/// rectangle — and the renderer computes a field's drawn area from those
/// rectangles, so the overhang was clipped. Scaling to the apothem that
/// inscribes puts the widest points on the box edge, like every other family.
fn sd_hexagon(point: [f32; 2], circumradius: f32) -> f32 {
    let radius = circumradius * 0.866_025_4;
    let k = [-0.866_025_4f32, 0.5, 0.577_350_26];
    let mut p = [point[0].abs(), point[1].abs()];
    let d = 2.0 * (k[0] * p[0] + k[1] * p[1]).min(0.0);
    p = [p[0] - d * k[0], p[1] - d * k[1]];
    p = [
        p[0] - p[0].clamp(-k[2] * radius, k[2] * radius),
        p[1] - radius,
    ];
    length(p) * p[1].signum()
}

/// A star with a whole number of points.
fn sd_star_n(point: [f32; 2], radius: f32, n: f32, inner: f32) -> f32 {
    let m = inner.clamp(0.02, 0.98);
    let an = std::f32::consts::PI / n;
    let en = std::f32::consts::PI / (2.0 + m * (n - 2.0)).max(2.001);
    let racs = [radius * an.cos(), radius * an.sin()];
    let ecs = [en.cos(), en.sin()];
    let bn = (point[0].abs()).atan2(point[1].max(-1e20)) % (2.0 * an) - an;
    let l = length(point);
    let mut p = [l * bn.cos(), l * bn.sin().abs()];
    p = [p[0] - racs[0], p[1] - racs[1]];
    let t = (-(p[0] * ecs[0] + p[1] * ecs[1])).clamp(0.0, racs[1] / ecs[1].max(0.0001));
    p = [p[0] + ecs[0] * t, p[1] + ecs[1] * t];
    length(p) * p[0].signum()
}

/// A star whose point count may be fractional.
///
/// A star is only defined for a whole number of points, so animating the count
/// through `floor` makes a new spike appear at full size between one frame and
/// the next. Blending the two neighbouring stars as *fields* instead grows the
/// new point out of the edge.
fn sd_star(point: [f32; 2], radius: f32, points: f32, inner: f32) -> f32 {
    let n = points.max(3.0);
    let lower = n.floor();
    let fraction = n - lower;
    let a = sd_star_n(point, radius, lower, inner);
    if fraction <= 0.0001 {
        return a;
    }
    let b = sd_star_n(point, radius, lower + 1.0, inner);
    a + (b - a) * fraction
}

fn sd_ring(point: [f32; 2], radius: f32, thickness: f32) -> f32 {
    let t = thickness.max(0.0001);
    (length(point) - radius + t * 0.5).abs() - t * 0.5
}

fn sd_pie(point: [f32; 2], radius: f32, degrees: f32) -> f32 {
    // Centred on straight up, so a growing angle opens symmetrically.
    let half_angle = degrees.clamp(0.0, 360.0) * 0.008_726_646;
    let c = [half_angle.sin(), half_angle.cos()];
    let p = [point[0].abs(), -point[1]];
    let l = length(p) - radius;
    let dot = (p[0] * c[0] + p[1] * c[1]).clamp(0.0, radius);
    let m = length([p[0] - c[0] * dot, p[1] - c[1] * dot]);
    l.max(m * (c[1] * p[0] - c[0] * p[1]).signum())
}

fn sd_cross(point: [f32; 2], half: [f32; 2], thickness: f32) -> f32 {
    let t = thickness.max(0.0001) * 0.5;
    let horizontal = sd_box_uniform(point, [half[0], t.min(half[1])], 0.0);
    let vertical = sd_box_uniform(point, [t.min(half[0]), half[1]], 0.0);
    horizontal.min(vertical)
}

/// An ellipse filling the box.
///
/// The exact ellipse distance needs a root solve; this scales the point into
/// the unit circle and back by the shorter half-extent instead. That makes the
/// sign exact — a point is inside exactly when the normalised radius is under
/// one — and the magnitude an underestimate that grows with eccentricity. The
/// sign is what an input region and a coverage boundary both actually read;
/// the magnitude only sets how wide the antialiasing ramp is.
fn sd_ellipse(point: [f32; 2], half: [f32; 2]) -> f32 {
    let rx = half[0].max(0.0001);
    let ry = half[1].max(0.0001);
    (length([point[0] / rx, point[1] / ry]) - 1.0) * rx.min(ry)
}

fn length(point: [f32; 2]) -> f32 {
    (point[0] * point[0] + point[1] * point[1]).sqrt()
}
