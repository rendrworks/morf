//! Raster, SVG, and XDG icon-theme loading with size-aware caches.

use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::ImageReader;
use resvg::{tiny_skia, usvg};

include!("quantize.rs");
include!("image_cache.rs");
include!("icons.rs");
#[cfg(test)]
mod tests;
