//! The boxes a fragment skips runs of outline with have to skip only runs that
//! could not have answered it.
//!
//! The shader walks a letter's edges per pixel, so it walks bounding boxes
//! first and opens a run only when the run could hold the nearest edge or a
//! crossing. That is a claim about arithmetic, not about a picture: a skipped
//! run is one whose box is further off than the nearest edge already found and
//! out of the band a rightward ray could cross. So the two walks are compared
//! here on real letters — the same measurement `field.wgsl` makes, transcribed
//! — and they must agree exactly, not nearly. A skipped run drops candidates
//! that were provably no better than the winner, which leaves the minimum
//! untouched, and contributes no crossings, which leaves the winding untouched.

use morf_layout::Geometry;
use morf_region::{Operation, Shape};
use morf_scene::Color;

use crate::commands::SdfLayer;
use crate::field::glyph_layer::{OUTLINE_SPAN, polygon_params};

/// Every edge measured, which is what the shader did before it had boxes.
fn walk_every_edge(points: &[[f32; 2]], at: (f32, f32), first: usize, stride: usize) -> f32 {
    let mut nearest = f32::MAX;
    let mut winding = 0i32;
    for index in 0..stride {
        let a = points[first + index];
        let b = points[first + (index + 1) % stride];
        nearest = nearest.min(square_distance(a, b, at));
        winding += crossing(a, b, at);
    }
    signed(nearest, winding)
}

/// The runs whose boxes rule them out are never opened.
fn walk_boxed(
    points: &[[f32; 2]],
    at: (f32, f32),
    first: usize,
    stride: usize,
    boxes: usize,
    opened: &mut usize,
) -> f32 {
    let mut nearest = f32::MAX;
    let mut winding = 0i32;
    let mut span = 0;
    while span * OUTLINE_SPAN < stride {
        let low = points[boxes + span * 2];
        let high = points[boxes + span * 2 + 1];
        let may_cross = at.1 >= low[1] && at.1 < high[1] && at.0 < high[0];
        let away = [
            (low[0] - at.0).max(at.0 - high[0]).max(0.0),
            (low[1] - at.1).max(at.1 - high[1]).max(0.0),
        ];
        if !may_cross && away[0] * away[0] + away[1] * away[1] >= nearest {
            span += 1;
            continue;
        }
        for step in 0..OUTLINE_SPAN {
            let index = span * OUTLINE_SPAN + step;
            if index >= stride {
                break;
            }
            let a = points[first + index];
            let b = points[first + (index + 1) % stride];
            nearest = nearest.min(square_distance(a, b, at));
            winding += crossing(a, b, at);
            *opened += 1;
        }
        span += 1;
    }
    signed(nearest, winding)
}

fn square_distance(a: [f32; 2], b: [f32; 2], at: (f32, f32)) -> f32 {
    let edge = [b[0] - a[0], b[1] - a[1]];
    let to_point = [at.0 - a[0], at.1 - a[1]];
    let length = (edge[0] * edge[0] + edge[1] * edge[1]).max(1e-9);
    let along = ((to_point[0] * edge[0] + to_point[1] * edge[1]) / length).clamp(0.0, 1.0);
    let offset = [to_point[0] - edge[0] * along, to_point[1] - edge[1] * along];
    offset[0] * offset[0] + offset[1] * offset[1]
}

fn crossing(a: [f32; 2], b: [f32; 2], at: (f32, f32)) -> i32 {
    if (a[1] <= at.1) == (b[1] <= at.1) {
        return 0;
    }
    let t = (at.1 - a[1]) / (b[1] - a[1]);
    if a[0] + t * (b[0] - a[0]) <= at.0 {
        return 0;
    }
    if b[1] > a[1] { 1 } else { -1 }
}

fn signed(nearest: f32, winding: i32) -> f32 {
    if winding != 0 {
        -nearest.sqrt()
    } else {
        nearest.sqrt()
    }
}

fn glyph_layer_for(glyph: char) -> SdfLayer {
    SdfLayer {
        glyph: Some(glyph),
        glyph_morph_to: None,
        svg_source: None,
        svg_source_morph_to: None,
        font_family: None,
        font_family_morph_to: None,
        bounds: Geometry {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 120.0,
        },
        color: Color::rgba8(255, 255, 255, 255),
        shape: Shape::Polygon,
        morph_to: Shape::Polygon,
        morph: 0.0,
        operation: Operation::Union,
        blend: 0.0,
        rotation: 0.0,
        radii: [0.0; 4],
        points: 5.0,
        inner_radius: 0.5,
        thickness: 0.0,
        angle: 90.0,
    }
}

#[test]
fn boxing_a_contour_skips_only_runs_that_could_not_have_won() {
    let stride = morf_text::GLYPH_CONTOUR_POINTS;
    let spans = stride.div_ceil(OUTLINE_SPAN);
    let mut text = morf_text::TextSystem::new();
    let mut drawings = morf_svg::SvgOutlines::new();
    let mut checked = 0;
    // Letters with counters, with a single stroke, and with several pieces —
    // the box walk has to hold for a contour whichever kind of shape it is.
    for glyph in "8B@gRo·il.,".chars() {
        let mut points = Vec::new();
        let (params, loops) = polygon_params(
            &glyph_layer_for(glyph),
            1.0,
            &mut points,
            &mut text,
            &mut drawings,
        );
        let loops = loops as usize;
        if loops == 0 {
            continue;
        }
        let first = params[0] as usize;
        assert_eq!(points.len(), first + loops * (stride + spans * 2));
        for step_y in 0..48 {
            for step_x in 0..48 {
                // Beyond the letter as well as across it: a fragment outside a
                // field's shape is measured too, and it is the one with the
                // most runs to skip.
                let at = (-90.0 + step_x as f32 * 3.75, -90.0 + step_y as f32 * 3.75);
                for contour in 0..loops {
                    let start = first + contour * stride;
                    let boxes = first + loops * stride + contour * spans * 2;
                    assert_eq!(
                        walk_boxed(&points, at, start, stride, boxes, &mut 0),
                        walk_every_edge(&points, at, start, stride),
                        "{glyph} contour {contour} at {at:?}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 10_000, "no letter had an outline to walk");
}

/// What the boxes are for, in the only terms that matter: how much of a letter
/// a fragment still has to walk. Run it with
/// `cargo test -p morf-render -- --ignored --nocapture opens`.
#[test]
#[ignore = "a measurement, not a claim"]
fn boxing_a_contour_opens_a_fraction_of_it() {
    let stride = morf_text::GLYPH_CONTOUR_POINTS;
    let spans = stride.div_ceil(OUTLINE_SPAN);
    let mut text = morf_text::TextSystem::new();
    let mut drawings = morf_svg::SvgOutlines::new();
    for glyph in "8B@gRoil".chars() {
        let mut points = Vec::new();
        let (params, loops) = polygon_params(
            &glyph_layer_for(glyph),
            1.0,
            &mut points,
            &mut text,
            &mut drawings,
        );
        let loops = loops as usize;
        if loops == 0 {
            continue;
        }
        let first = params[0] as usize;
        let (mut opened, mut every) = (0usize, 0usize);
        for step_y in 0..120 {
            for step_x in 0..120 {
                // The layer's own box, which is what a field actually shades.
                let at = (-60.0 + step_x as f32, -60.0 + step_y as f32);
                for contour in 0..loops {
                    let start = first + contour * stride;
                    let boxes = first + loops * stride + contour * spans * 2;
                    walk_boxed(&points, at, start, stride, boxes, &mut opened);
                    every += stride;
                }
            }
        }
        let share = opened as f64 / every as f64 * 100.0;
        println!("{glyph}: {loops} contours, {share:.1}% of edges opened");
    }
}
