//! Composable surface regions, over the shape vocabulary in [`shapes`].
//!
//! Every family the renderer can draw can be composed into an input region
//! here, by the same analytic distance function, so a star-shaped node is
//! clickable as a star rather than as the rectangle that contains it.

use std::error::Error;
use std::fmt;

pub mod shapes;

pub use shapes::{Operation, Shape, ShapeParams, combine, distance};

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

/// One shape placed on a surface, and how it joins what came before it.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub rect: Rect,
    pub shape: Shape,
    /// The family's own parameters — corner radii, star points, ring
    /// thickness. Flat rather than inside the `Shape` variants so that this is
    /// the same layout the renderer packs into a layer uniform.
    pub params: ShapeParams,
    pub operation: Operation,
    pub children: Vec<Region>,
}

impl Default for Region {
    /// A plain rectangle covering its own bounds.
    ///
    /// Not `Shape::default()`, which is a circle: a region that names no shape
    /// means the whole rectangle it was given, and always has. The shape
    /// enum's own default belongs to the renderer, where a bare field is a
    /// circle.
    fn default() -> Self {
        Self {
            rect: Rect::default(),
            shape: Shape::Box,
            params: ShapeParams::default(),
            operation: Operation::default(),
            children: Vec::new(),
        }
    }
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
    // A mask has no partial coverage, so a smooth seam has no room to exist
    // here: the shapes are the same, and every pixel of the seam has to be in
    // or out either way.
    let operation = region.operation.hard();
    if !reaches(region, window) {
        if operation == Operation::Intersect {
            mask.fill(false);
        }
        return Ok(());
    }
    if region.children.is_empty() {
        stamp(
            window,
            region.rect,
            region.shape,
            &region.params,
            operation,
            mask,
        );
        return Ok(());
    }
    let mut own = vec![false; mask.len()];
    stamp(
        window,
        region.rect,
        region.shape,
        &region.params,
        Operation::Union,
        &mut own,
    );
    for child in &region.children {
        compose(window, child, &mut own, depth + 1)?;
    }
    apply(mask, &own, operation);
    Ok(())
}

/// Writes one shape onto a mask under `operation`, in the window's frame.
fn stamp(
    window: Rect,
    rect: Rect,
    shape: Shape,
    params: &ShapeParams,
    operation: Operation,
    mask: &mut [bool],
) {
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
                }) && contains(rect, shape, params, x, y);
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
    let square = shape.fills_box(params);
    // A plain rectangle covers whole rows, and most regions are plain
    // rectangles. Writing a row at a time skips the per-pixel corner and
    // ellipse arithmetic entirely.
    if square && operation != Operation::Xor {
        let value = operation == Operation::Union;
        for y in area.y..area.y + area.height {
            let row = (y - window.y) as usize * stride;
            let start = row + (area.x - window.x) as usize;
            mask[start..start + area.width as usize].fill(value);
        }
        return;
    }
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if !contains(rect, shape, params, x, y) {
                continue;
            }
            let index = (y - window.y) as usize * stride + (x - window.x) as usize;
            mask[index] = match operation {
                Operation::Union => true,
                Operation::Subtract => false,
                Operation::Xor => !mask[index],
                Operation::Intersect => unreachable!("handled above"),
                smooth => unreachable!("{} was hardened by compose", smooth.name()),
            };
        }
    }
}

/// Whether the pixel at `(x, y)` is covered by the shape in `rect`.
///
/// The pixel is sampled at its own centre, and coverage is the sign of the
/// same distance function the shader evaluates — so a click lands exactly
/// where the shape was drawn, for every family, rather than only for the two
/// the region rasteriser used to know.
fn contains(rect: Rect, shape: Shape, params: &ShapeParams, x: i32, y: i32) -> bool {
    let half = [
        f64::from(rect.width.max(0)) as f32 / 2.0,
        f64::from(rect.height.max(0)) as f32 / 2.0,
    ];
    if half[0] <= 0.0 || half[1] <= 0.0 {
        return false;
    }
    let point = [
        (x - rect.x) as f32 + 0.5 - half[0],
        (y - rect.y) as f32 + 0.5 - half[1],
    ];
    distance(shape, params, half, point) <= 0.0
}

fn apply(target: &mut [bool], source: &[bool], operation: Operation) {
    for (target, source) in target.iter_mut().zip(source) {
        *target = match operation {
            Operation::Union => *target || *source,
            Operation::Subtract => *target && !*source,
            Operation::Intersect => *target && *source,
            Operation::Xor => *target != *source,
            smooth => unreachable!("{} was hardened by compose", smooth.name()),
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
mod tests;
