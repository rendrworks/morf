use mold_scene::{Element, NodeHandle};

/// Logical dimensions in surface coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
}

/// Resolved logical geometry for one node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Geometry {
    /// Horizontal offset from the parent.
    pub x: f64,
    /// Vertical offset from the parent.
    pub y: f64,
    /// Resolved width.
    pub width: f64,
    /// Resolved height.
    pub height: f64,
}

/// Surface-space affine transform stored as two rows of a 3x3 matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform2D {
    pub matrix: [f64; 6],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformParameters {
    pub translation: [f64; 2],
    pub scale: [f64; 2],
    pub rotation: f64,
    pub skew: [f64; 2],
}

impl Default for TransformParameters {
    fn default() -> Self {
        Self {
            translation: [0.0; 2],
            scale: [1.0; 2],
            rotation: 0.0,
            skew: [0.0; 2],
        }
    }
}

impl Default for Transform2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform2D {
    pub const IDENTITY: Self = Self {
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };

    /// Builds a uniform scale and clockwise rotation around a surface point.
    pub fn around(center: (f64, f64), scale: f64, rotation_degrees: f64) -> Self {
        Self::affine(
            center,
            TransformParameters {
                scale: [scale; 2],
                rotation: rotation_degrees,
                ..TransformParameters::default()
            },
        )
    }

    /// Builds translation, non-uniform scale, skew, and rotation around a surface point.
    pub fn affine(origin: (f64, f64), parameters: TransformParameters) -> Self {
        let radians = parameters.rotation.to_radians();
        let cosine = radians.cos();
        let sine = radians.sin();
        let skew_x = parameters.skew[0].to_radians().tan();
        let skew_y = parameters.skew[1].to_radians().tan();
        let scale_x = parameters.scale[0];
        let scale_y = parameters.scale[1];
        let a = scale_x * (cosine - sine * skew_y);
        let b = scale_x * (sine + cosine * skew_y);
        let c = scale_y * (cosine * skew_x - sine);
        let d = scale_y * (sine * skew_x + cosine);
        let (x, y) = origin;
        Self {
            matrix: [
                a,
                b,
                c,
                d,
                x - a * x - c * y + parameters.translation[0],
                y - b * x - d * y + parameters.translation[1],
            ],
        }
    }

    /// Composes this transform after `inner`.
    pub fn then(self, inner: Self) -> Self {
        let [a, b, c, d, tx, ty] = self.matrix;
        let [e, f, g, h, ux, uy] = inner.matrix;
        Self {
            matrix: [
                a * e + c * f,
                b * e + d * f,
                a * g + c * h,
                b * g + d * h,
                a * ux + c * uy + tx,
                b * ux + d * uy + ty,
            ],
        }
    }

    pub fn point(self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, tx, ty] = self.matrix;
        (a * x + c * y + tx, b * x + d * y + ty)
    }

    pub fn inverse_point(self, x: f64, y: f64) -> Option<(f64, f64)> {
        let [a, b, c, d, tx, ty] = self.matrix;
        let determinant = a * d - b * c;
        if determinant.abs() <= f64::EPSILON {
            return None;
        }
        let x = x - tx;
        let y = y - ty;
        Some((
            (d * x - c * y) / determinant,
            (-b * x + a * y) / determinant,
        ))
    }

    pub fn bounds(self, geometry: Geometry) -> Geometry {
        let points = [
            self.point(geometry.x, geometry.y),
            self.point(geometry.x + geometry.width, geometry.y),
            self.point(geometry.x, geometry.y + geometry.height),
            self.point(geometry.x + geometry.width, geometry.y + geometry.height),
        ];
        let min_x = points
            .iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min);
        let min_y = points
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min);
        let max_x = points
            .iter()
            .map(|point| point.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = points
            .iter()
            .map(|point| point.1)
            .fold(f64::NEG_INFINITY, f64::max);
        Geometry {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        }
    }
}

/// Horizontal positioning applied while shaping text lines.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextAlignment {
    #[default]
    Left,
    Right,
    Center,
    Justified,
}

/// Ellipsis placement when an unwrapped line exceeds its width.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextElide {
    #[default]
    None,
    Left,
    Middle,
    Right,
}

/// Width, wrapping, and alignment supplied to the text subsystem.
#[derive(Clone, Debug, PartialEq)]
pub struct TextOptions {
    pub width: Option<f64>,
    pub wrap: bool,
    pub alignment: TextAlignment,
    pub elide: TextElide,
    pub font_weight: f64,
    pub font_source: Option<String>,
}

impl Default for TextOptions {
    fn default() -> Self {
        Self {
            width: None,
            wrap: false,
            alignment: TextAlignment::Left,
            elide: TextElide::None,
            font_weight: 400.0,
            font_source: None,
        }
    }
}

/// Text measurement supplied by the text subsystem.
pub trait TextMeasurer {
    /// Shapes text and returns its logical bounds.
    fn measure(
        &mut self,
        node: NodeHandle,
        text: &str,
        family: &str,
        size: f64,
        options: TextOptions,
    ) -> Size;

    /// Returns intrinsic image or icon dimensions when the source is available.
    fn measure_image(
        &mut self,
        _node: NodeHandle,
        _element: Element,
        _source: &str,
        _theme: Option<&str>,
    ) -> Option<Size> {
        None
    }
}
