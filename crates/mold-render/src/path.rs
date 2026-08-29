use std::collections::HashMap;

use lyon_tessellation::geom::{Angle, ArcFlags};
use lyon_tessellation::math::{point, vector};
use lyon_tessellation::path::Path;
use lyon_tessellation::path::builder::SvgPathBuilder;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use svgtypes::{PathParser, PathSegment};

#[derive(Clone, Debug, Default)]
pub(crate) struct Mesh {
    pub(crate) vertices: Vec<[f32; 2]>,
    pub(crate) indices: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PathMesh {
    pub(crate) fill: Mesh,
    pub(crate) stroke: Mesh,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PathKey {
    data: String,
    stroke_width: u64,
    even_odd: bool,
    scale_120: u32,
}

#[derive(Default)]
pub(crate) struct PathCache {
    meshes: HashMap<PathKey, PathMesh>,
}

impl PathCache {
    pub(crate) fn tessellate(
        &mut self,
        data: &str,
        stroke_width: f64,
        even_odd: bool,
        scale_120: u32,
    ) -> Result<&PathMesh, String> {
        let key = PathKey {
            data: data.to_owned(),
            stroke_width: stroke_width.to_bits(),
            even_odd,
            scale_120,
        };
        if !self.meshes.contains_key(&key) {
            let path = parse_path(data)?;
            let scale = scale_120.max(1) as f32 / 120.0;
            let tolerance = 0.1 / scale;
            let mut fill = VertexBuffers::new();
            let fill_rule = if even_odd {
                FillRule::EvenOdd
            } else {
                FillRule::NonZero
            };
            FillTessellator::new()
                .tessellate_path(
                    &path,
                    &FillOptions::tolerance(tolerance).with_fill_rule(fill_rule),
                    &mut BuffersBuilder::new(&mut fill, |vertex: FillVertex<'_>| {
                        vertex.position().to_array()
                    }),
                )
                .map_err(|error| format!("could not tessellate path fill: {error}"))?;
            let mut stroke = VertexBuffers::new();
            if stroke_width > 0.0 {
                StrokeTessellator::new()
                    .tessellate_path(
                        &path,
                        &StrokeOptions::tolerance(tolerance).with_line_width(stroke_width as f32),
                        &mut BuffersBuilder::new(&mut stroke, |vertex: StrokeVertex<'_, '_>| {
                            vertex.position().to_array()
                        }),
                    )
                    .map_err(|error| format!("could not tessellate path stroke: {error}"))?;
            }
            self.meshes.insert(
                key.clone(),
                PathMesh {
                    fill: Mesh {
                        vertices: fill.vertices,
                        indices: fill.indices,
                    },
                    stroke: Mesh {
                        vertices: stroke.vertices,
                        indices: stroke.indices,
                    },
                },
            );
        }
        Ok(&self.meshes[&key])
    }
}

fn parse_path(data: &str) -> Result<Path, String> {
    let mut builder = Path::builder().with_svg();
    for segment in PathParser::from(data) {
        match segment.map_err(|error| format!("invalid SVG path: {error}"))? {
            PathSegment::MoveTo { abs, x, y } if abs => {
                builder.move_to(point(x as f32, y as f32));
            }
            PathSegment::MoveTo { x, y, .. } => {
                builder.relative_move_to(vector(x as f32, y as f32));
            }
            PathSegment::LineTo { abs, x, y } if abs => {
                builder.line_to(point(x as f32, y as f32));
            }
            PathSegment::LineTo { x, y, .. } => {
                builder.relative_line_to(vector(x as f32, y as f32));
            }
            PathSegment::HorizontalLineTo { abs: true, x } => {
                builder.horizontal_line_to(x as f32);
            }
            PathSegment::HorizontalLineTo { x, .. } => {
                builder.relative_horizontal_line_to(x as f32);
            }
            PathSegment::VerticalLineTo { abs: true, y } => {
                builder.vertical_line_to(y as f32);
            }
            PathSegment::VerticalLineTo { y, .. } => {
                builder.relative_vertical_line_to(y as f32);
            }
            PathSegment::CurveTo {
                abs: true,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                builder.cubic_bezier_to(
                    point(x1 as f32, y1 as f32),
                    point(x2 as f32, y2 as f32),
                    point(x as f32, y as f32),
                );
            }
            PathSegment::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
                ..
            } => {
                builder.relative_cubic_bezier_to(
                    vector(x1 as f32, y1 as f32),
                    vector(x2 as f32, y2 as f32),
                    vector(x as f32, y as f32),
                );
            }
            PathSegment::SmoothCurveTo {
                abs: true,
                x2,
                y2,
                x,
                y,
            } => {
                builder
                    .smooth_cubic_bezier_to(point(x2 as f32, y2 as f32), point(x as f32, y as f32));
            }
            PathSegment::SmoothCurveTo { x2, y2, x, y, .. } => {
                builder.smooth_relative_cubic_bezier_to(
                    vector(x2 as f32, y2 as f32),
                    vector(x as f32, y as f32),
                );
            }
            PathSegment::Quadratic {
                abs: true,
                x1,
                y1,
                x,
                y,
            } => {
                builder.quadratic_bezier_to(point(x1 as f32, y1 as f32), point(x as f32, y as f32));
            }
            PathSegment::Quadratic { x1, y1, x, y, .. } => {
                builder.relative_quadratic_bezier_to(
                    vector(x1 as f32, y1 as f32),
                    vector(x as f32, y as f32),
                );
            }
            PathSegment::SmoothQuadratic { abs: true, x, y } => {
                builder.smooth_quadratic_bezier_to(point(x as f32, y as f32));
            }
            PathSegment::SmoothQuadratic { x, y, .. } => {
                builder.smooth_relative_quadratic_bezier_to(vector(x as f32, y as f32));
            }
            PathSegment::EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                let radii = vector(rx as f32, ry as f32);
                let rotation = Angle::degrees(x_axis_rotation as f32);
                let flags = ArcFlags { large_arc, sweep };
                if abs {
                    builder.arc_to(radii, rotation, flags, point(x as f32, y as f32));
                } else {
                    builder.relative_arc_to(radii, rotation, flags, vector(x as f32, y as f32));
                }
            }
            PathSegment::ClosePath { .. } => builder.close(),
        }
    }
    Ok(builder.build())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
