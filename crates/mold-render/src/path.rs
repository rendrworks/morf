use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use lyon_tessellation::geom::{Angle, ArcFlags};
use lyon_tessellation::math::{point, vector};
use lyon_tessellation::path::Path;
use lyon_tessellation::path::builder::SvgPathBuilder;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use svgtypes::{PathParser, PathSegment};

use mold_layout::Geometry;
use mold_scene::NodeHandle;
use polymorpher::geometry::Size;
use polymorpher::{CornerRounding, Morph, RoundedPolygon, shapes};

use crate::ShapeMorph;

include!("path/shapes.rs");

/// Triangles in path coordinates, each vertex carrying its coverage position.
///
/// The third vertex component is `SOLID` for interior geometry and runs from
/// `0` to `1` across the antialiasing band that skirts the outline.
#[derive(Clone, Debug, Default)]
pub(crate) struct Mesh {
    pub(crate) vertices: Vec<[f32; 3]>,
    pub(crate) indices: Vec<u32>,
}

/// Coverage marker for geometry that is fully inside the shape.
const SOLID: f32 = -2.0;

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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MorphKey {
    from: String,
    to: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MorphedMeshKey {
    morph: MorphKey,
    progress: u32,
    width: u32,
    height: u32,
    stroke_width: u64,
    even_odd: bool,
    scale_120: u32,
}

#[derive(Default)]
pub(crate) struct PathCache {
    meshes: HashMap<PathKey, PathMesh>,
    morphs: HashMap<MorphKey, Morph>,
    morphed: HashMap<NodeHandle, (MorphedMeshKey, PathMesh)>,
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
            let mesh = tessellate_path(&path, stroke_width, even_odd, scale_120)?;
            self.meshes.insert(key.clone(), mesh);
        }
        Ok(&self.meshes[&key])
    }

    pub(crate) fn tessellate_morph(
        &mut self,
        node: NodeHandle,
        spec: &ShapeMorph,
        bounds: Geometry,
        stroke_width: f64,
        even_odd: bool,
        scale_120: u32,
    ) -> Result<&PathMesh, String> {
        let morph = MorphKey {
            from: spec.from.clone(),
            to: spec.to.clone(),
        };
        let key = MorphedMeshKey {
            morph: morph.clone(),
            progress: spec.progress.to_bits(),
            width: (bounds.width as f32).to_bits(),
            height: (bounds.height as f32).to_bits(),
            stroke_width: stroke_width.to_bits(),
            even_odd,
            scale_120,
        };
        let current = self.morphed.get(&node).map(|(current, _)| current);
        if current != Some(&key) {
            if !self.morphs.contains_key(&morph) {
                let start = morph_shape(&morph.from)?;
                let end = morph_shape(&morph.to)?;
                self.morphs.insert(morph.clone(), Morph::new(start, end));
            }
            let path = morph_path(
                &self.morphs[&morph],
                spec.progress,
                bounds.width as f32,
                bounds.height as f32,
            );
            let mesh = tessellate_path(&path, stroke_width, even_odd, scale_120)?;
            self.morphed.insert(node, (key, mesh));
        }
        Ok(&self.morphed[&node].1)
    }

    pub(crate) fn retain_morphs(&mut self, nodes: &HashSet<NodeHandle>) {
        self.morphed.retain(|node, _| nodes.contains(node));
    }
}

fn tessellate_path(
    path: &Path,
    stroke_width: f64,
    even_odd: bool,
    scale_120: u32,
) -> Result<PathMesh, String> {
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
            path,
            &FillOptions::tolerance(tolerance).with_fill_rule(fill_rule),
            &mut BuffersBuilder::new(&mut fill, |vertex: FillVertex<'_>| {
                let [x, y] = vertex.position().to_array();
                [x, y, SOLID]
            }),
        )
        .map_err(|error| format!("could not tessellate path fill: {error}"))?;
    let mut stroke = VertexBuffers::new();
    if stroke_width > 0.0 {
        StrokeTessellator::new()
            .tessellate_path(
                path,
                &StrokeOptions::tolerance(tolerance).with_line_width(stroke_width as f32),
                &mut BuffersBuilder::new(&mut stroke, |vertex: StrokeVertex<'_, '_>| {
                    let [x, y] = vertex.position().to_array();
                    [x, y, SOLID]
                }),
            )
            .map_err(|error| format!("could not tessellate path stroke: {error}"))?;
    }
    // One physical pixel expressed in the path's own coordinates, so the band
    // stays a pixel wide whatever scale the geometry was tessellated for.
    let fringe = 1.0 / scale;
    let mut fill = Mesh {
        vertices: fill.vertices,
        indices: fill.indices,
    };
    let mut stroke = Mesh {
        vertices: stroke.vertices,
        indices: stroke.indices,
    };
    add_coverage_band(&mut fill, fringe);
    add_coverage_band(&mut stroke, fringe);
    Ok(PathMesh { fill, stroke })
}

/// Skirts a tessellated mesh with a one-pixel outward coverage band.
///
/// A tessellator emits hard polygon edges and the rasterizer samples them once
/// per pixel, which is why an untreated curve comes out visibly stepped. The
/// band fades from full coverage at the outline to nothing a pixel beyond it.
///
/// It is built from the mesh rather than from the path, which matters twice
/// over: the outline is found as the edges belonging to a single triangle, so
/// the result does not depend on how the contours were wound, and the band is
/// extruded strictly outwards, so it never blends over the shape it is
/// smoothing and a translucent fill stays exactly as translucent as it was
/// asked to be. Strokes are closed regions too, so the same pass covers them.
fn add_coverage_band(mesh: &mut Mesh, width: f32) {
    if mesh.indices.len() < 3 || !width.is_finite() || width <= 0.0 {
        return;
    }
    // An interior edge is shared by two triangles; anything seen once is on the
    // outline. The opposite corner is kept to tell inwards from outwards.
    let mut edges: HashMap<(u32, u32), (u32, u32, u32)> = HashMap::new();
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        for (from, to, opposite) in [(a, b, c), (b, c, a), (c, a, b)] {
            let key = (from.min(to), from.max(to));
            match edges.entry(key) {
                Entry::Occupied(entry) => {
                    entry.remove();
                }
                Entry::Vacant(entry) => {
                    entry.insert((from, to, opposite));
                }
            }
        }
    }
    if edges.is_empty() {
        return;
    }

    // Outward normals are averaged per vertex so the band closes at corners
    // instead of leaving a wedge between neighbouring edges.
    fn point(vertices: &[[f32; 3]], index: u32) -> (f32, f32) {
        let vertex = vertices[index as usize];
        (vertex[0], vertex[1])
    }
    let mut normals: HashMap<u32, (f32, f32)> = HashMap::new();
    let boundary = edges.values().copied().collect::<Vec<_>>();
    for (from, to, opposite) in &boundary {
        let (ax, ay) = point(&mesh.vertices, *from);
        let (bx, by) = point(&mesh.vertices, *to);
        let (dx, dy) = (bx - ax, by - ay);
        let length = dx.hypot(dy);
        if length <= f32::EPSILON {
            continue;
        }
        let mut normal = (dy / length, -dx / length);
        let (cx, cy) = point(&mesh.vertices, *opposite);
        let midpoint = ((ax + bx) * 0.5, (ay + by) * 0.5);
        if normal.0 * (cx - midpoint.0) + normal.1 * (cy - midpoint.1) > 0.0 {
            normal = (-normal.0, -normal.1);
        }
        for vertex in [*from, *to] {
            let entry = normals.entry(vertex).or_insert((0.0, 0.0));
            entry.0 += normal.0;
            entry.1 += normal.1;
        }
    }

    // The outline vertices are duplicated rather than reused: the originals
    // stay marked solid for the fill triangles that share them, while the
    // copies carry the band's coverage ramp.
    let mut ring: HashMap<u32, (u32, u32)> = HashMap::new();
    for (vertex, normal) in &normals {
        let length = normal.0.hypot(normal.1);
        if length <= f32::EPSILON {
            continue;
        }
        let (nx, ny) = (normal.0 / length * width, normal.1 / length * width);
        let (x, y) = point(&mesh.vertices, *vertex);
        let inner = mesh.vertices.len() as u32;
        mesh.vertices.push([x, y, 0.0]);
        mesh.vertices.push([x + nx, y + ny, 1.0]);
        ring.insert(*vertex, (inner, inner + 1));
    }

    for (from, to, _) in &boundary {
        let (Some((inner_from, outer_from)), Some((inner_to, outer_to))) =
            (ring.get(from).copied(), ring.get(to).copied())
        else {
            continue;
        };
        mesh.indices
            .extend_from_slice(&[inner_from, inner_to, outer_to]);
        mesh.indices
            .extend_from_slice(&[inner_from, outer_to, outer_from]);
    }
}

fn morph_path(morph: &Morph, progress: f32, width: f32, height: f32) -> Path {
    let cubics = morph.as_cubics(progress.clamp(0.0, 1.0));
    let mut builder = Path::builder();
    if let Some(first) = cubics.first() {
        let anchor = first.anchor0();
        builder.begin(point(anchor.x * width, anchor.y * height));
        for cubic in cubics {
            let control0 = cubic.control0();
            let control1 = cubic.control1();
            let anchor1 = cubic.anchor1();
            builder.cubic_bezier_to(
                point(control0.x * width, control0.y * height),
                point(control1.x * width, control1.y * height),
                point(anchor1.x * width, anchor1.y * height),
            );
        }
        builder.end(true);
    }
    builder.build()
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
mod tests;
