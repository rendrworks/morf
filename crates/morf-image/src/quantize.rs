use std::fs;
use std::path::Path;

use resvg::usvg;

use crate::image_cache::{ImageError, decode_path, normalize_source};

/// Decoded straight-alpha RGBA pixels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageData {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Row-major RGBA8 pixels.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Extracts up to `2^depth` prevalent opaque colors from an image.
pub fn quantize_colors(
    source: impl AsRef<Path>,
    depth: u8,
    crop: Option<ImageRect>,
    rescale_size: u32,
) -> Result<Vec<[u8; 4]>, ImageError> {
    if depth > 8 || rescale_size > 512 {
        return Err(ImageError::InvalidSize);
    }
    let source = normalize_source(source.as_ref())?;
    let (intrinsic_width, intrinsic_height) =
        if source.extension().and_then(|value| value.to_str()) == Some("svg") {
            let bytes = fs::read(&source)?;
            let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default())
                .map_err(|error| ImageError::Svg(error.to_string()))?;
            let size = tree.size();
            (size.width().ceil() as u32, size.height().ceil() as u32)
        } else {
            image::image_dimensions(&source)?
        };
    if intrinsic_width == 0
        || intrinsic_height == 0
        || u64::from(intrinsic_width) * u64::from(intrinsic_height) > 16_777_216
    {
        return Err(ImageError::InvalidSize);
    }
    let target = if rescale_size == 0 {
        (intrinsic_width, intrinsic_height)
    } else if intrinsic_width >= intrinsic_height {
        (
            rescale_size,
            (u64::from(intrinsic_height) * u64::from(rescale_size) / u64::from(intrinsic_width))
                .max(1) as u32,
        )
    } else {
        (
            (u64::from(intrinsic_width) * u64::from(rescale_size) / u64::from(intrinsic_height))
                .max(1) as u32,
            rescale_size,
        )
    };
    let image = decode_path(&source, target.0, target.1)?;
    let crop = crop.map(|crop| ImageRect {
        x: (u64::from(crop.x) * u64::from(target.0) / u64::from(intrinsic_width)) as u32,
        y: (u64::from(crop.y) * u64::from(target.1) / u64::from(intrinsic_height)) as u32,
        width: (u64::from(crop.width) * u64::from(target.0) / u64::from(intrinsic_width)).max(1)
            as u32,
        height: (u64::from(crop.height) * u64::from(target.1) / u64::from(intrinsic_height)).max(1)
            as u32,
    });
    quantize_image(&image, depth, crop)
}

pub(crate) fn quantize_image(
    image: &ImageData,
    depth: u8,
    crop: Option<ImageRect>,
) -> Result<Vec<[u8; 4]>, ImageError> {
    let crop = crop.unwrap_or(ImageRect {
        x: 0,
        y: 0,
        width: image.width,
        height: image.height,
    });
    let right = crop.x.saturating_add(crop.width).min(image.width);
    let bottom = crop.y.saturating_add(crop.height).min(image.height);
    if crop.x >= right || crop.y >= bottom {
        return Err(ImageError::InvalidSize);
    }
    let mut pixels = Vec::with_capacity(((right - crop.x) * (bottom - crop.y)) as usize);
    for y in crop.y..bottom {
        for x in crop.x..right {
            let offset = ((y * image.width + x) * 4) as usize;
            let pixel = &image.rgba[offset..offset + 4];
            if pixel[3] != 0 {
                pixels.push([pixel[0], pixel[1], pixel[2], pixel[3]]);
            }
        }
    }
    if pixels.is_empty() {
        return Ok(Vec::new());
    }
    let mut buckets = vec![pixels];
    for _ in 0..depth {
        let mut next = Vec::with_capacity(buckets.len() * 2);
        for mut bucket in buckets {
            if bucket.len() < 2 {
                next.push(bucket);
                continue;
            }
            let channel = widest_channel(&bucket);
            bucket.sort_unstable_by_key(|pixel| pixel[channel]);
            let second = bucket.split_off(bucket.len() / 2);
            next.push(bucket);
            next.push(second);
        }
        buckets = next;
    }
    Ok(buckets
        .into_iter()
        .filter(|bucket| !bucket.is_empty())
        .map(|bucket| {
            let mut sums = [0_u64; 4];
            for pixel in &bucket {
                for channel in 0..4 {
                    sums[channel] += u64::from(pixel[channel]);
                }
            }
            let length = bucket.len() as u64;
            [
                (sums[0] / length) as u8,
                (sums[1] / length) as u8,
                (sums[2] / length) as u8,
                (sums[3] / length) as u8,
            ]
        })
        .collect())
}

fn widest_channel(pixels: &[[u8; 4]]) -> usize {
    let mut minimum = [u8::MAX; 3];
    let mut maximum = [u8::MIN; 3];
    for pixel in pixels {
        for channel in 0..3 {
            minimum[channel] = minimum[channel].min(pixel[channel]);
            maximum[channel] = maximum[channel].max(pixel[channel]);
        }
    }
    (0..3)
        .max_by_key(|channel| maximum[*channel] - minimum[*channel])
        .unwrap_or(0)
}
