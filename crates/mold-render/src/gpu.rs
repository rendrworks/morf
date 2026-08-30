use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt;
use std::mem;
use std::ops::Range;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::util::DeviceExt;

use mold_image::ImageCache;
use mold_layout::{Geometry, Size, TextMeasurer, TextOptions, Transform2D};
use mold_scene::{Color, Element, NodeHandle};
use mold_text::{RasterContent, RasterGlyph, TextSystem};

use crate::path::PathCache;
use crate::{
    DamageRect, DrawCommand, DrawList, ImageFillMode, LayerMask, RenderBackend, SdfQuadInstance,
    VerticalAlignment, color_array, physical_damage,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

include!("gpu/backend_types.rs");
include!("gpu/backend_init.rs");
include!("gpu/backend_render.rs");
include!("gpu/path_batch.rs");
include!("gpu/glyphs.rs");
include!("gpu/pipelines.rs");
include!("gpu/textures.rs");
include!("gpu/targets.rs");
#[cfg(test)]
mod tests;
