//! What an outline is, without any particular source of one.

use super::*;

/// A square, described four ways, is the same four loops of points.
///
/// The point of the crate: whoever hands over the outline decides nothing.
#[test]
fn a_shape_is_the_same_shape_however_it_was_drawn() {
    let square = [
        Step::Move(0.0, 0.0),
        Step::Line(10.0, 0.0),
        Step::Line(10.0, 10.0),
        Step::Line(0.0, 10.0),
        Step::Close,
    ];
    // The same square with the closing edge written out instead of implied, and
    // with a curve that happens to be straight.
    let spelled_out = [
        Step::Move(0.0, 0.0),
        Step::Line(10.0, 0.0),
        Step::Line(10.0, 10.0),
        Step::Line(0.0, 10.0),
        Step::Line(0.0, 0.0),
        Step::Close,
    ];
    let curved = [
        Step::Move(0.0, 0.0),
        Step::Quad(5.0, 0.0, 10.0, 0.0),
        Step::Cubic(10.0, 3.0, 10.0, 7.0, 10.0, 10.0),
        Step::Line(0.0, 10.0),
        Step::Close,
    ];

    for outline in [square.as_slice(), &spelled_out, &curved] {
        let loops = contours(outline);
        assert_eq!(loops.len(), 1, "one closed loop");
        assert_eq!(contour_of(&loops[0]).len(), CONTOUR_POINTS);
        // Every point is on the square's boundary.
        for (x, y) in contour_of(&loops[0]) {
            let on_edge = x.abs() < 0.01
                || (x - 10.0).abs() < 0.01
                || y.abs() < 0.01
                || (y - 10.0).abs() < 0.01;
            assert!(on_edge, "({x}, {y}) is not on the square");
        }
    }
}

/// An unclosed subpath is closed anyway.
///
/// A distance is measured to a boundary, and a boundary with a gap in it is not
/// one: the winding count escapes through the gap and the shape comes back
/// inside out.
#[test]
fn an_outline_left_open_is_closed_for_it() {
    let open = [
        Step::Move(0.0, 0.0),
        Step::Line(10.0, 0.0),
        Step::Line(10.0, 10.0),
        Step::Line(0.0, 10.0),
    ];
    let pieces = flatten(&open);
    let last = pieces.last().expect("an outline with edges in it");
    assert!(
        (last.x1, last.y1) == (0.0, 0.0),
        "the last piece returns to where the first one started"
    );
}

/// Two outlines with different numbers of loops still correspond.
///
/// One loop of the first is matched to one of the second by where they sit and
/// how they are wound; a loop with no opposite number walks to a point rather
/// than to nothing, so the extra counter of an `8` closes rather than vanishing.
#[test]
fn outlines_with_different_loop_counts_still_pair_up() {
    let ring = |inset: f32| {
        vec![
            Step::Move(inset, inset),
            Step::Line(20.0 - inset, inset),
            Step::Line(20.0 - inset, 20.0 - inset),
            Step::Line(inset, 20.0 - inset),
            Step::Close,
        ]
    };
    let mut hollow = ring(0.0);
    // A counter, wound the other way.
    hollow.extend([
        Step::Move(6.0, 6.0),
        Step::Line(6.0, 14.0),
        Step::Line(14.0, 14.0),
        Step::Line(14.0, 6.0),
        Step::Close,
    ]);

    let paired = pair_up(contours(&hollow), contours(&ring(2.0)));
    assert_eq!(
        paired.len(),
        2,
        "both loops of the fuller shape are carried"
    );
    let start = walk(&paired, 0.0);
    let end = walk(&paired, 1.0);
    let half = walk(&paired, 0.5);
    assert_eq!(start.len(), end.len());
    assert_eq!(half.len(), start.len());
    assert_ne!(start, end);
    assert_ne!(half, start);
    assert_ne!(half, end);
}
