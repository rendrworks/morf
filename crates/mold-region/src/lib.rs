//! Composable rectangular, rounded, and elliptical surface regions.

use std::error::Error;
use std::fmt;

const MAX_PIXELS: usize = 16_777_216;
const MAX_RECTS: usize = 65_536;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shape {
    Rectangle {
        top_left: u32,
        top_right: u32,
        bottom_right: u32,
        bottom_left: u32,
    },
    Ellipse,
}

impl Default for Shape {
    fn default() -> Self {
        Self::Rectangle {
            top_left: 0,
            top_right: 0,
            bottom_right: 0,
            bottom_left: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Operation {
    #[default]
    Combine,
    Subtract,
    Intersect,
    Xor,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Region {
    pub rect: Rect,
    pub shape: Shape,
    pub operation: Operation,
    pub children: Vec<Region>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionError {
    SurfaceTooLarge,
    TooDeep,
    TooComplex,
}

impl fmt::Display for RegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SurfaceTooLarge => "region surface exceeds 16777216 pixels",
            Self::TooDeep => "region nesting exceeds 64 levels",
            Self::TooComplex => "region output exceeds 65536 rectangles",
        })
    }
}

impl Error for RegionError {}

/// Composes `regions` into the disjoint rectangles a compositor input region
/// wants.
///
/// The work is proportional to the area the regions actually cover, not to the
/// surface. A shell whose interactive parts are a bar and two panels pays for a
/// bar and two panels, however large the output is — which matters because this
/// runs on every paint of every surface.
pub fn build(width: u32, height: u32, regions: &[Region]) -> Result<Vec<Rect>, RegionError> {
    usize::try_from(u64::from(width) * u64::from(height))
        .ok()
        .filter(|length| *length <= MAX_PIXELS)
        .ok_or(RegionError::SurfaceTooLarge)?;
    let surface = Rect {
        x: 0,
        y: 0,
        width: width as i32,
        height: height as i32,
    };
    // Every operation here is pointwise, so composing the tree over any subset
    // of the surface gives the same answer as composing it whole and then
    // restricting. A pixel can only end up set if some region's own rectangle
    // covers it, at whatever depth, so those rectangles — merged where they
    // overlap — are a set of windows that misses nothing.
    let mut bounds = Vec::new();
    for region in regions {
        collect(region, surface, 0, &mut bounds)?;
    }
    let mut rects = Vec::new();
    let windows = merge(bounds);
    for window in windows {
        // Every top-level region is replayed against every window. One that
        // does not reach this window contributes nothing under `Combine`,
        // `Subtract` and `Xor` — but `Intersect` against an empty mask clears
        // the window, which is exactly what intersecting with a shape that is
        // somewhere else means.
        let area = window.width as usize * window.height as usize;
        let mut mask = vec![false; area];
        for region in regions {
            compose(window, region, &mut mask, 0)?;
        }
        rectangles(window, &mask, &mut rects)?;
    }
    rects.sort_by_key(|rect| (rect.y, rect.x));
    Ok(rects)
}

/// Intersects a region rectangle with a window, in the window's own frame.
fn clip(rect: Rect, window: Rect) -> Option<Rect> {
    let left = rect.x.max(window.x);
    let top = rect.y.max(window.y);
    let right = rect
        .x
        .saturating_add(rect.width)
        .min(window.x + window.width);
    let bottom = rect
        .y
        .saturating_add(rect.height)
        .min(window.y + window.height);
    (left < right && top < bottom).then_some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

/// Gathers every rectangle in the tree that falls inside `window`.
fn collect(
    region: &Region,
    window: Rect,
    depth: usize,
    into: &mut Vec<Rect>,
) -> Result<(), RegionError> {
    if depth >= MAX_DEPTH {
        return Err(RegionError::TooDeep);
    }
    if let Some(rect) = clip(region.rect, window) {
        into.push(rect);
    }
    for child in &region.children {
        collect(child, window, depth + 1, into)?;
    }
    Ok(())
}

/// Whether a region or anything under it can reach inside `window`.
fn reaches(region: &Region, window: Rect) -> bool {
    clip(region.rect, window).is_some()
        || region.children.iter().any(|child| reaches(child, window))
}

/// The smallest rectangle containing both, when there is one.
fn span(left: Option<Rect>, right: Option<Rect>) -> Option<Rect> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let x = left.x.min(right.x);
            let y = left.y.min(right.y);
            Some(Rect {
                x,
                y,
                width: (left.x + left.width).max(right.x + right.width) - x,
                height: (left.y + left.height).max(right.y + right.height) - y,
            })
        }
        (found, None) | (None, found) => found,
    }
}

/// Collapses overlapping bounds so no two windows cover the same pixel twice.
fn merge(mut bounds: Vec<Rect>) -> Vec<Rect> {
    let mut merged: Vec<Rect> = Vec::new();
    while let Some(mut rect) = bounds.pop() {
        let mut absorbed = true;
        while absorbed {
            absorbed = false;
            let mut index = 0;
            while index < merged.len() {
                if overlaps(rect, merged[index]) {
                    rect = span(Some(rect), Some(merged.swap_remove(index))).unwrap_or(rect);
                    absorbed = true;
                } else {
                    index += 1;
                }
            }
        }
        merged.push(rect);
    }
    merged
}

fn overlaps(left: Rect, right: Rect) -> bool {
    left.x < right.x + right.width
        && right.x < left.x + left.width
        && left.y < right.y + right.height
        && right.y < left.y + left.height
}

/// Applies one region, and everything under it, onto `mask`.
///
/// A region with no children of its own is written straight into the target,
/// so the common case — a plain interactive rectangle — costs its own area and
/// nothing else. Only a region that composes children needs scratch space, and
/// only `Intersect` has to touch the parts of the window it does not cover.
fn compose(
    window: Rect,
    region: &Region,
    mask: &mut [bool],
    depth: usize,
) -> Result<(), RegionError> {
    if depth >= MAX_DEPTH {
        return Err(RegionError::TooDeep);
    }
    if !reaches(region, window) {
        if region.operation == Operation::Intersect {
            mask.fill(false);
        }
        return Ok(());
    }
    if region.children.is_empty() {
        stamp(window, region.rect, region.shape, region.operation, mask);
        return Ok(());
    }
    let mut own = vec![false; mask.len()];
    stamp(
        window,
        region.rect,
        region.shape,
        Operation::Combine,
        &mut own,
    );
    for child in &region.children {
        compose(window, child, &mut own, depth + 1)?;
    }
    apply(mask, &own, region.operation);
    Ok(())
}

/// Writes one shape onto a mask under `operation`, in the window's frame.
fn stamp(window: Rect, rect: Rect, shape: Shape, operation: Operation, mask: &mut [bool]) {
    let stride = window.width as usize;
    let area = clip(rect, window);
    if operation == Operation::Intersect {
        for y in window.y..window.y + window.height {
            for x in window.x..window.x + window.width {
                let inside = area.is_some_and(|area| {
                    x >= area.x
                        && y >= area.y
                        && x < area.x + area.width
                        && y < area.y + area.height
                }) && contains(rect, shape, x, y);
                if !inside {
                    mask[(y - window.y) as usize * stride + (x - window.x) as usize] = false;
                }
            }
        }
        return;
    }
    let Some(area) = area else {
        return;
    };
    let square = matches!(
        shape,
        Shape::Rectangle {
            top_left: 0,
            top_right: 0,
            bottom_right: 0,
            bottom_left: 0,
        }
    );
    // A plain rectangle covers whole rows, and most regions are plain
    // rectangles. Writing a row at a time skips the per-pixel corner and
    // ellipse arithmetic entirely.
    if square && operation != Operation::Xor {
        let value = operation == Operation::Combine;
        for y in area.y..area.y + area.height {
            let row = (y - window.y) as usize * stride;
            let start = row + (area.x - window.x) as usize;
            mask[start..start + area.width as usize].fill(value);
        }
        return;
    }
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if !contains(rect, shape, x, y) {
                continue;
            }
            let index = (y - window.y) as usize * stride + (x - window.x) as usize;
            mask[index] = match operation {
                Operation::Combine => true,
                Operation::Subtract => false,
                Operation::Xor => !mask[index],
                Operation::Intersect => unreachable!("handled above"),
            };
        }
    }
}

fn contains(rect: Rect, shape: Shape, x: i32, y: i32) -> bool {
    let local_x = f64::from(x - rect.x) + 0.5;
    let local_y = f64::from(y - rect.y) + 0.5;
    let width = f64::from(rect.width.max(0));
    let height = f64::from(rect.height.max(0));
    match shape {
        Shape::Ellipse => {
            let radius_x = width / 2.0;
            let radius_y = height / 2.0;
            radius_x > 0.0
                && radius_y > 0.0
                && ((local_x - radius_x) / radius_x).powi(2)
                    + ((local_y - radius_y) / radius_y).powi(2)
                    <= 1.0
        }
        Shape::Rectangle {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } => {
            corner_contains(local_x, local_y, width, height, top_left, 0)
                && corner_contains(local_x, local_y, width, height, top_right, 1)
                && corner_contains(local_x, local_y, width, height, bottom_right, 2)
                && corner_contains(local_x, local_y, width, height, bottom_left, 3)
        }
    }
}

fn corner_contains(x: f64, y: f64, width: f64, height: f64, radius: u32, corner: u8) -> bool {
    let radius = f64::from(radius).min(width / 2.0).min(height / 2.0);
    if radius == 0.0 {
        return true;
    }
    let (center_x, center_y, relevant) = match corner {
        0 => (radius, radius, x < radius && y < radius),
        1 => (width - radius, radius, x > width - radius && y < radius),
        2 => (
            width - radius,
            height - radius,
            x > width - radius && y > height - radius,
        ),
        _ => (radius, height - radius, x < radius && y > height - radius),
    };
    !relevant || (x - center_x).powi(2) + (y - center_y).powi(2) <= radius.powi(2)
}

fn apply(target: &mut [bool], source: &[bool], operation: Operation) {
    for (target, source) in target.iter_mut().zip(source) {
        *target = match operation {
            Operation::Combine => *target || *source,
            Operation::Subtract => *target && !*source,
            Operation::Intersect => *target && *source,
            Operation::Xor => *target != *source,
        };
    }
}

/// Merges the mask's set pixels into vertical runs, appending them to `rects`.
///
/// Coordinates come back in the surface's frame, not the window's.
/// Merges the mask's set pixels into vertical runs, appending them to `rects`.
///
/// Coordinates come back in the surface's frame, not the window's. Runs within
/// a row arrive in x order, so a row is matched against the one above it by
/// walking both in step rather than by hashing.
fn rectangles(window: Rect, mask: &[bool], rects: &mut Vec<Rect>) -> Result<(), RegionError> {
    let stride = window.width as usize;
    let mut active: Vec<((i32, i32), Rect)> = Vec::new();
    let mut runs: Vec<(i32, i32)> = Vec::new();
    let mut next: Vec<((i32, i32), Rect)> = Vec::new();
    for y in 0..window.height {
        let row = y as usize * stride;
        runs.clear();
        let mut x = 0;
        while x < window.width {
            if !mask[row + x as usize] {
                x += 1;
                continue;
            }
            let start = x;
            while x < window.width && mask[row + x as usize] {
                x += 1;
            }
            runs.push((start, x - start));
        }
        next.clear();
        let mut open = active.drain(..).peekable();
        let mut fresh = runs.iter().copied().peekable();
        loop {
            match (open.peek(), fresh.peek()) {
                (Some((key, _)), Some(run)) if key == run => {
                    let (key, mut rect) = open.next().expect("peeked");
                    fresh.next();
                    rect.height += 1;
                    next.push((key, rect));
                }
                (Some((key, _)), Some(run)) if key < run => {
                    rects.push(open.next().expect("peeked").1);
                }
                (_, Some(run)) => {
                    next.push((
                        *run,
                        Rect {
                            x: window.x + run.0,
                            y: window.y + y,
                            width: run.1,
                            height: 1,
                        },
                    ));
                    fresh.next();
                }
                (Some(_), None) => rects.push(open.next().expect("peeked").1),
                (None, None) => break,
            }
        }
        drop(open);
        if rects.len() + next.len() > MAX_RECTS {
            return Err(RegionError::TooComplex);
        }
        active.append(&mut next);
    }
    rects.extend(active.into_iter().map(|(_, rect)| rect));
    Ok(())
}

#[cfg(test)]
include!("tests.rs");
