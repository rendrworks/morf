//! Composable rectangular, rounded, and elliptical surface regions.

use std::collections::HashMap;
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

pub fn build(width: u32, height: u32, regions: &[Region]) -> Result<Vec<Rect>, RegionError> {
    let length = usize::try_from(u64::from(width) * u64::from(height))
        .ok()
        .filter(|length| *length <= MAX_PIXELS)
        .ok_or(RegionError::SurfaceTooLarge)?;
    let mut mask = vec![false; length];
    for region in regions {
        let child = rasterize(width, height, region, 0)?;
        apply(&mut mask, &child, region.operation);
    }
    rectangles(width, height, &mask)
}

fn rasterize(
    width: u32,
    height: u32,
    region: &Region,
    depth: usize,
) -> Result<Vec<bool>, RegionError> {
    if depth >= MAX_DEPTH {
        return Err(RegionError::TooDeep);
    }
    let mut mask = vec![false; width as usize * height as usize];
    let left = region.rect.x.max(0).min(width as i32);
    let top = region.rect.y.max(0).min(height as i32);
    let right = region
        .rect
        .x
        .saturating_add(region.rect.width)
        .max(0)
        .min(width as i32);
    let bottom = region
        .rect
        .y
        .saturating_add(region.rect.height)
        .max(0)
        .min(height as i32);
    if left < right && top < bottom {
        for y in top..bottom {
            for x in left..right {
                if contains(region.rect, region.shape, x, y) {
                    mask[y as usize * width as usize + x as usize] = true;
                }
            }
        }
    }
    for child in &region.children {
        let child_mask = rasterize(width, height, child, depth + 1)?;
        apply(&mut mask, &child_mask, child.operation);
    }
    Ok(mask)
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

fn rectangles(width: u32, height: u32, mask: &[bool]) -> Result<Vec<Rect>, RegionError> {
    let mut completed = Vec::new();
    let mut active = HashMap::<(i32, i32), Rect>::new();
    for y in 0..height as i32 {
        let mut runs = Vec::new();
        let mut x = 0;
        while x < width as i32 {
            if !mask[y as usize * width as usize + x as usize] {
                x += 1;
                continue;
            }
            let start = x;
            while x < width as i32 && mask[y as usize * width as usize + x as usize] {
                x += 1;
            }
            runs.push((start, x - start));
        }
        let mut next = HashMap::new();
        for run in runs {
            let rect = active.remove(&run).map_or(
                Rect {
                    x: run.0,
                    y,
                    width: run.1,
                    height: 1,
                },
                |mut rect| {
                    rect.height += 1;
                    rect
                },
            );
            next.insert(run, rect);
        }
        completed.extend(active.into_values());
        if completed.len() + next.len() > MAX_RECTS {
            return Err(RegionError::TooComplex);
        }
        active = next;
    }
    completed.extend(active.into_values());
    completed.sort_by_key(|rect| (rect.y, rect.x));
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(x: i32, y: i32, width: i32, height: i32, operation: Operation) -> Region {
        Region {
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            operation,
            ..Region::default()
        }
    }

    #[test]
    fn combines_subtracts_and_merges_vertical_runs() {
        let regions = [Region {
            rect: Rect {
                x: 0,
                y: 0,
                width: 6,
                height: 4,
            },
            children: vec![rectangle(2, 1, 2, 2, Operation::Subtract)],
            ..Region::default()
        }];
        assert_eq!(
            build(6, 4, &regions).unwrap(),
            [
                Rect {
                    x: 0,
                    y: 0,
                    width: 6,
                    height: 1
                },
                Rect {
                    x: 0,
                    y: 1,
                    width: 2,
                    height: 2
                },
                Rect {
                    x: 4,
                    y: 1,
                    width: 2,
                    height: 2
                },
                Rect {
                    x: 0,
                    y: 3,
                    width: 6,
                    height: 1
                },
            ]
        );
    }

    #[test]
    fn ellipse_and_xor_are_composable() {
        let ellipse = Region {
            rect: Rect {
                x: 0,
                y: 0,
                width: 5,
                height: 5,
            },
            shape: Shape::Ellipse,
            ..Region::default()
        };
        let regions = [
            ellipse.clone(),
            Region {
                operation: Operation::Xor,
                ..ellipse
            },
        ];
        assert!(build(5, 5, &regions).unwrap().is_empty());
    }
}
