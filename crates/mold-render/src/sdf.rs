/// Instance payload consumed by the SDF quad shader.
#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy, Debug, PartialEq)]
pub struct SdfQuadInstance {
    /// Physical x, y, width, and height.
    pub bounds: [f32; 4],
    /// RGBA fill.
    pub color: [f32; 4],
    /// Per-corner radii in clockwise order.
    pub radii: [f32; 4],
    /// Border width followed by padding.
    pub border: [f32; 4],
    /// RGBA border.
    pub border_color: [f32; 4],
    /// Original rectangle within the expanded effect bounds.
    pub shape: [f32; 4],
    /// Fill blur, shadow blur, shadow spread, and padding.
    pub effects: [f32; 4],
    /// Shadow offset followed by padding.
    pub shadow: [f32; 4],
    /// RGBA shadow colour.
    pub shadow_color: [f32; 4],
    /// RGBA gradient start colour.
    pub gradient_start_color: [f32; 4],
    /// RGBA gradient end colour.
    pub gradient_end_color: [f32; 4],
    /// Normalized gradient start and end points.
    pub gradient_points: [f32; 4],
    /// Gradient kind, center, and radius.
    pub gradient_data: [f32; 4],
    /// RGBA colour overlay.
    pub color_overlay: [f32; 4],
    /// Affine linear terms in column order.
    pub transform: [f32; 4],
    /// Affine translation in logical surface coordinates.
    pub transform_offset: [f32; 2],
}

impl SdfQuadInstance {
    /// Converts a quad command to physical GPU instance data.
    pub fn from_command(command: &DrawCommand, scale_120: u32) -> Option<Self> {
        let DrawCommand::Quad {
            bounds,
            transform,
            color,
            color_overlay,
            gradient,
            radii,
            border_width,
            antialiasing,
            border_pixel_aligned,
            border_color,
            blur,
            shadow_color,
            shadow_blur,
            shadow_spread,
            shadow_offset_x,
            shadow_offset_y,
            shadow_inner,
            ..
        } = command
        else {
            return None;
        };
        let scale = scale_120.max(1) as f64 / 120.0;
        let expanded = effect_bounds(
            *bounds,
            *blur,
            if *shadow_inner { 0.0 } else { *shadow_blur },
            if *shadow_inner { 0.0 } else { *shadow_spread },
            if *shadow_inner { 0.0 } else { *shadow_offset_x },
            if *shadow_inner { 0.0 } else { *shadow_offset_y },
        );
        let (gradient_start_color, gradient_end_color, gradient_points, gradient_data, angle) =
            gradient_instance(gradient);
        Some(Self {
            bounds: [
                (expanded.x * scale) as f32,
                (expanded.y * scale) as f32,
                (expanded.width * scale) as f32,
                (expanded.height * scale) as f32,
            ],
            color: color_array(*color),
            radii: radii.map(|radius| (radius.max(0.0) * scale) as f32),
            border: [
                if *border_pixel_aligned {
                    (*border_width * scale).round() as f32
                } else {
                    (*border_width * scale) as f32
                },
                if *antialiasing { 1.0 } else { 0.0 },
                0.0,
                0.0,
            ],
            border_color: color_array(*border_color),
            shape: [
                ((bounds.x - expanded.x) * scale) as f32,
                ((bounds.y - expanded.y) * scale) as f32,
                (bounds.width * scale) as f32,
                (bounds.height * scale) as f32,
            ],
            effects: [
                (*blur * scale) as f32,
                (*shadow_blur * scale) as f32,
                (*shadow_spread * scale) as f32,
                angle,
            ],
            shadow: [
                (*shadow_offset_x * scale) as f32,
                (*shadow_offset_y * scale) as f32,
                if *shadow_inner { 1.0 } else { 0.0 },
                0.0,
            ],
            shadow_color: color_array(*shadow_color),
            gradient_start_color,
            gradient_end_color,
            gradient_points,
            gradient_data,
            color_overlay: color_array(*color_overlay),
            transform: [
                transform.matrix[0] as f32,
                transform.matrix[1] as f32,
                transform.matrix[2] as f32,
                transform.matrix[3] as f32,
            ],
            transform_offset: [
                (transform.matrix[4] * scale) as f32,
                (transform.matrix[5] * scale) as f32,
            ],
        })
    }
}

/// Scene or backend failure while producing a frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// Scene property or handle failure.
    Scene(String),
    /// Selected rendering backend failure.
    Backend(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(message) => write!(f, "scene paint error: {message}"),
            Self::Backend(message) => write!(f, "render backend error: {message}"),
        }
    }
}

impl StdError for RenderError {}

impl From<SceneError> for RenderError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error.to_string())
    }
}
