//! Raster, SVG, and XDG icon-theme loading with size-aware caches.

mod distance_field;
mod icons;
mod image_cache;
mod quantize;

pub use icons::IconResolver;
pub use image_cache::{ImageCache, ImageError};
pub use quantize::{ImageData, ImageRect, quantize_colors};
#[cfg(test)]
mod tests;
