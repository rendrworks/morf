//! What comes out of a document, and what can be done with it afterwards.

use morf_outline::{CONTOUR_POINTS, contour_of, contours, pair_up, walk};

use super::*;

const RING: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <path fill-rule="evenodd" d="M10 10 H90 V90 H10 Z M30 30 H70 V70 H30 Z"/>
</svg>"#;

const LINE_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"
  fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
  <path d="M4 12 H20"/>
</svg>"#;

#[test]
fn a_document_arrives_as_loops_of_points_and_not_as_pixels() {
    let outline = outline_from_bytes(RING.as_bytes()).unwrap();
    assert_eq!((outline.width, outline.height), (100.0, 100.0));
    let loops = contours(&outline.steps);
    assert_eq!(loops.len(), 2, "the square and the hole in it");
    for contour in &loops {
        assert_eq!(contour_of(contour).len(), CONTOUR_POINTS);
    }
}

/// `evenodd` and a winding count are different rules, and the second is the one
/// a field has. The hole has to come back wound against the square it is in, or
/// the two cancel the wrong way and the shape fills solid.
#[test]
fn an_even_odd_hole_is_wound_against_what_encloses_it() {
    let outline = outline_from_bytes(RING.as_bytes()).unwrap();
    let loops = contours(&outline.steps);
    let area = |contour: &morf_outline::Contour| {
        let points = contour_of(contour);
        let mut total = 0.0;
        for index in 0..points.len() {
            let (x0, y0) = points[index];
            let (x1, y1) = points[(index + 1) % points.len()];
            total += x0 * y1 - x1 * y0;
        }
        total * 0.5
    };
    let mut areas: Vec<f32> = loops.iter().map(area).collect();
    areas.sort_by(f32::total_cmp);
    assert!(
        areas[0] < 0.0 && areas[1] > 0.0,
        "one loop is wound each way, so the hole cancels the body: {areas:?}"
    );
}

/// A line icon has no fill at all — what is on screen is the stroke, so the
/// stroke has to become an outline of its own or there is nothing to measure.
#[test]
fn a_stroke_with_no_fill_still_has_an_edge() {
    let outline = outline_from_bytes(LINE_ICON.as_bytes()).unwrap();
    let loops = contours(&outline.steps);
    assert_eq!(loops.len(), 1, "the widened stroke, as one closed loop");
    let points = contour_of(&loops[0]);
    let widest = points
        .iter()
        .fold(f32::MIN, |widest, (_, y)| widest.max(*y));
    let narrowest = points.iter().fold(f32::MAX, |low, (_, y)| low.min(*y));
    // Two units wide, plus the round caps, and nowhere near the 24 of the box.
    assert!(
        (widest - narrowest - 2.0).abs() < 0.5,
        "the stroke is as thick as it was asked to be: {narrowest}..{widest}"
    );
}

/// The reason for all of it: once a drawing is an outline, it is the same kind
/// of thing as every other outline and can be walked onto one.
#[test]
fn a_drawing_and_a_shape_walk_onto_each_other() {
    let outline = outline_from_bytes(RING.as_bytes()).unwrap();
    let circle = [
        Step::Move(50.0, 10.0),
        Step::Cubic(72.0, 10.0, 90.0, 28.0, 90.0, 50.0),
        Step::Cubic(90.0, 72.0, 72.0, 90.0, 50.0, 90.0),
        Step::Cubic(28.0, 90.0, 10.0, 72.0, 10.0, 50.0),
        Step::Cubic(10.0, 28.0, 28.0, 10.0, 50.0, 10.0),
        Step::Close,
    ];
    let paired = pair_up(contours(&outline.steps), contours(&circle));
    let start = walk(&paired, 0.0);
    let half = walk(&paired, 0.5);
    let end = walk(&paired, 1.0);
    assert_eq!(
        start.len(),
        end.len(),
        "one correspondence, one point count"
    );
    assert_eq!(half.len(), start.len());
    assert_ne!(start, end);
    assert_ne!(half, start);
    assert_ne!(half, end);
}
