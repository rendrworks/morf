//! Rasterising a region on a coarser grid.
//!
//! Split from `lib` because it is a different bargain, not a different
//! implementation: [`build`](crate::build) answers exactly, and this answers to
//! within a cell in exchange for the square of the saving. Whether that is a
//! good trade is the caller's to know, so the two are named apart.

use crate::{Rect, Region, RegionError, ShapeParams, build};

/// The divisor to use when the caller paints its own antialiased edge over the
/// region's boundary — a backdrop blur, a shadow.
///
/// Eight pixels is sixty-four times less work than one, and the boundary it
/// leaves is eight pixels coarse: invisible under a painted edge, and wrong
/// anywhere the region's own edge is what gets seen.
pub const COVERED_EDGE_GRID: u32 = 8;

/// Builds a region on a grid `divisor` times coarser, and scales it back.
///
/// Rasterising is O(area) and a caller that rebuilds a full-screen region every
/// frame pays that every frame: measured at six milliseconds for six circles on
/// a 3456x2160 surface, in release, against a frame budget of sixteen. A
/// divisor of four is sixteen times less work.
///
/// What it costs is a boundary accurate to `divisor` pixels *in either
/// direction*. Rectangles round outward, so a region never shrinks wholesale —
/// but a shape sampled on a coarser grid can still miss a sliver a cell wide,
/// which is what happens along the tangent of a circle. Anything more than a
/// cell inside the fine region is covered; the last cell is a coin toss.
///
/// That is the right trade wherever the caller paints its own antialiased edge
/// over the boundary — a blur region, a shadow — and the wrong one wherever the
/// region's edge is itself the product, which is why this is a separate
/// function and [`build`] still means what it said.
pub fn build_scaled(
    width: u32,
    height: u32,
    regions: &[Region],
    divisor: u32,
) -> Result<Vec<Rect>, RegionError> {
    let divisor = divisor.max(1);
    if divisor == 1 {
        return build(width, height, regions);
    }
    let step = divisor as i32;
    let coarse: Vec<Region> = regions.iter().map(|region| shrink(region, step)).collect();
    let rects = build(width.div_ceil(divisor), height.div_ceil(divisor), &coarse)?;
    Ok(rects
        .into_iter()
        .map(|rect| Rect {
            x: rect.x * step,
            y: rect.y * step,
            width: rect.width * step,
            height: rect.height * step,
        })
        .collect())
}

/// One region and its children on a grid `step` times coarser.
///
/// Rectangles round outward, so the coarse region covers the fine one. A blur
/// that stops a pixel short of the shape drawn over it shows a hard edge; one
/// that overshoots by a pixel shows nothing at all.
fn shrink(region: &Region, step: i32) -> Region {
    let left = region.rect.x.div_euclid(step);
    let top = region.rect.y.div_euclid(step);
    let right = (region.rect.x + region.rect.width + step - 1).div_euclid(step);
    let bottom = (region.rect.y + region.rect.height + step - 1).div_euclid(step);
    let scale = step as f32;
    Region {
        rect: Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        },
        shape: region.shape,
        params: ShapeParams {
            radii: region.params.radii.map(|radius| radius / scale),
            thickness: region.params.thickness / scale,
            ..region.params
        },
        operation: region.operation,
        children: region
            .children
            .iter()
            .map(|child| shrink(child, step))
            .collect(),
    }
}
