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
use polymorpher::{Morph, RoundedPolygon, shapes};

use crate::ShapeMorph;

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
                vertex.position().to_array()
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
                    vertex.position().to_array()
                }),
            )
            .map_err(|error| format!("could not tessellate path stroke: {error}"))?;
    }
    Ok(PathMesh {
        fill: Mesh {
            vertices: fill.vertices,
            indices: fill.indices,
        },
        stroke: Mesh {
            vertices: stroke.vertices,
            indices: stroke.indices,
        },
    })
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

pub(crate) fn is_morph_shape(name: &str) -> bool {
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
    )
}

fn morph_shape(name: &str) -> Result<RoundedPolygon, String> {
    let shape = match name {
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
}
