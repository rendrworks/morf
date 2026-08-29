//! Raster, SVG, and XDG icon-theme loading with size-aware caches.

use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::ImageReader;
use resvg::{tiny_skia, usvg};

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

/// Image or icon loading failure.
#[derive(Debug)]
pub enum ImageError {
    /// The source could not be read.
    Io(std::io::Error),
    /// The raster format could not be decoded.
    Raster(image::ImageError),
    /// The SVG document could not be parsed or rasterized.
    Svg(String),
    /// No matching icon was found.
    IconNotFound(String),
    /// The requested output size was invalid.
    InvalidSize,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "image I/O failed: {error}"),
            Self::Raster(error) => write!(f, "image decode failed: {error}"),
            Self::Svg(error) => write!(f, "SVG decode failed: {error}"),
            Self::IconNotFound(name) => write!(f, "icon `{name}` was not found"),
            Self::InvalidSize => f.write_str("image size must be greater than zero"),
        }
    }
}

impl StdError for ImageError {}

impl From<std::io::Error> for ImageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<image::ImageError> for ImageError {
    fn from(error: image::ImageError) -> Self {
        Self::Raster(error)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    source: PathBuf,
    width: u32,
    height: u32,
    scale_120: u32,
}

/// Cache for decoded images and resolved icon names.
#[derive(Default)]
pub struct ImageCache {
    images: HashMap<CacheKey, Arc<ImageData>>,
    icons: HashMap<(String, String, u32, u32), PathBuf>,
    intrinsic: HashMap<PathBuf, (u32, u32)>,
}

impl ImageCache {
    /// Loads a source at a logical size and protocol scale in 120ths.
    pub fn load(
        &mut self,
        source: impl AsRef<Path>,
        logical_width: u32,
        logical_height: u32,
        scale_120: u32,
    ) -> Result<Arc<ImageData>, ImageError> {
        let source = source.as_ref().to_path_buf();
        let width = physical_size(logical_width, scale_120)?;
        let height = physical_size(logical_height, scale_120)?;
        let key = CacheKey {
            source: source.clone(),
            width,
            height,
            scale_120,
        };
        if let Some(image) = self.images.get(&key) {
            return Ok(Arc::clone(image));
        }
        let image = Arc::new(decode_path(&source, width, height)?);
        self.images.insert(key, Arc::clone(&image));
        Ok(image)
    }

    /// Resolves and loads an icon through XDG theme inheritance.
    pub fn load_icon(
        &mut self,
        name: &str,
        theme: &str,
        logical_size: u32,
        scale_120: u32,
    ) -> Result<Arc<ImageData>, ImageError> {
        self.load_icon_sized(name, theme, logical_size, logical_size, scale_120)
    }

    /// Resolves and loads an icon into a logical rectangle.
    pub fn load_icon_sized(
        &mut self,
        name: &str,
        theme: &str,
        logical_width: u32,
        logical_height: u32,
        scale_120: u32,
    ) -> Result<Arc<ImageData>, ImageError> {
        let logical_size = logical_width.max(logical_height);
        let physical = physical_size(logical_size, scale_120)?;
        let key = (name.to_owned(), theme.to_owned(), physical, scale_120);
        let path = if let Some(path) = self.icons.get(&key) {
            path.clone()
        } else {
            let path = IconResolver::from_environment().find(name, theme, physical)?;
            self.icons.insert(key, path.clone());
            path
        };
        self.load(path, logical_width, logical_height, scale_120)
    }

    /// Returns a source's unscaled pixel dimensions.
    pub fn intrinsic_size(&mut self, source: impl AsRef<Path>) -> Result<(u32, u32), ImageError> {
        let source = source.as_ref().to_path_buf();
        if let Some(size) = self.intrinsic.get(&source) {
            return Ok(*size);
        }
        let size = if source.extension().and_then(|value| value.to_str()) == Some("svg") {
            let bytes = fs::read(&source)?;
            let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default())
                .map_err(|error| ImageError::Svg(error.to_string()))?;
            let size = tree.size();
            (size.width().ceil() as u32, size.height().ceil() as u32)
        } else {
            image::image_dimensions(&source)?
        };
        if size.0 == 0 || size.1 == 0 {
            return Err(ImageError::InvalidSize);
        }
        self.intrinsic.insert(source, size);
        Ok(size)
    }

    /// Resolves an icon and returns its source dimensions.
    pub fn icon_intrinsic_size(
        &mut self,
        name: &str,
        theme: &str,
        preferred_size: u32,
    ) -> Result<(u32, u32), ImageError> {
        let key = (name.to_owned(), theme.to_owned(), preferred_size, 120);
        let path = if let Some(path) = self.icons.get(&key) {
            path.clone()
        } else {
            let path = IconResolver::from_environment().find(name, theme, preferred_size)?;
            self.icons.insert(key, path.clone());
            path
        };
        self.intrinsic_size(path)
    }

    /// Removes all decoded and resolved entries.
    pub fn clear(&mut self) {
        self.images.clear();
        self.icons.clear();
        self.intrinsic.clear();
    }
}

fn physical_size(logical: u32, scale_120: u32) -> Result<u32, ImageError> {
    if logical == 0 || scale_120 == 0 {
        return Err(ImageError::InvalidSize);
    }
    Ok(logical.saturating_mul(scale_120).div_ceil(120))
}

fn decode_path(path: &Path, width: u32, height: u32) -> Result<ImageData, ImageError> {
    let bytes = fs::read(path)?;
    if path.extension().and_then(|value| value.to_str()) == Some("svg") {
        decode_svg(&bytes, width, height)
    } else {
        decode_raster(&bytes, width, height)
    }
}

fn decode_raster(bytes: &[u8], width: u32, height: u32) -> Result<ImageData, ImageError> {
    let image = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?
        .resize_exact(width, height, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    Ok(ImageData {
        width,
        height,
        rgba: image.into_raw(),
    })
}

fn decode_svg(bytes: &[u8], width: u32, height: u32) -> Result<ImageData, ImageError> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
        .map_err(|error| ImageError::Svg(error.to_string()))?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(ImageError::InvalidSize)?;
    let size = tree.size();
    let transform = tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    Ok(ImageData {
        width,
        height,
        rgba,
    })
}

/// XDG icon-theme resolver with inheritance and size matching.
#[derive(Clone, Debug)]
pub struct IconResolver {
    roots: Vec<PathBuf>,
    pixmaps: Vec<PathBuf>,
}

impl IconResolver {
    /// Creates a resolver over explicit icon-theme roots.
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            pixmaps: Vec::new(),
        }
    }

    /// Creates a resolver from XDG data directories and the legacy user icon root.
    pub fn from_environment() -> Self {
        let home = env::var_os("HOME").map(PathBuf::from);
        let data_home = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|path| path.join(".local/share")));
        let data_dirs =
            env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let mut roots = Vec::new();
        let mut pixmaps = Vec::new();
        if let Some(data_home) = data_home {
            roots.push(data_home.join("icons"));
        }
        if let Some(home) = home {
            roots.push(home.join(".icons"));
        }
        for root in env::split_paths(&data_dirs) {
            roots.push(root.join("icons"));
            pixmaps.push(root.join("pixmaps"));
        }
        Self { roots, pixmaps }
    }

    /// Adds a non-themed pixmap fallback directory.
    pub fn with_pixmaps(mut self, path: PathBuf) -> Self {
        self.pixmaps.push(path);
        self
    }

    /// Finds the closest icon file for a physical pixel size.
    pub fn find(&self, name: &str, theme: &str, size: u32) -> Result<PathBuf, ImageError> {
        let mut visited = HashSet::new();
        if let Some(path) = self.find_theme(name, theme, size, &mut visited) {
            return Ok(path);
        }
        if theme != "hicolor"
            && let Some(path) = self.find_theme(name, "hicolor", size, &mut visited)
        {
            return Ok(path);
        }
        for root in &self.pixmaps {
            if let Some(path) = find_named_file(root, name) {
                return Ok(path);
            }
        }
        Err(ImageError::IconNotFound(name.to_owned()))
    }

    fn find_theme(
        &self,
        name: &str,
        theme: &str,
        size: u32,
        visited: &mut HashSet<String>,
    ) -> Option<PathBuf> {
        if !visited.insert(theme.to_owned()) {
            return None;
        }
        let mut inherited = Vec::new();
        let mut candidates = Vec::new();
        for root in &self.roots {
            let theme_root = root.join(theme);
            let Some(index) = ThemeIndex::load(&theme_root.join("index.theme")) else {
                continue;
            };
            inherited.extend(index.inherits.iter().cloned());
            for directory in index.directories {
                if let Some(path) = find_named_file(&theme_root.join(&directory.name), name) {
                    candidates.push((directory.distance(size), path));
                }
            }
        }
        if let Some((_, path)) = candidates.into_iter().min_by_key(|(distance, _)| *distance) {
            return Some(path);
        }
        for parent in inherited {
            if let Some(path) = self.find_theme(name, &parent, size, visited) {
                return Some(path);
            }
        }
        None
    }
}

#[derive(Default)]
struct ThemeIndex {
    inherits: Vec<String>,
    directories: Vec<IconDirectory>,
}

impl ThemeIndex {
    fn load(path: &Path) -> Option<Self> {
        let source = fs::read_to_string(path).ok()?;
        let sections = parse_ini(&source);
        let theme = sections.get("Icon Theme")?;
        let inherits = split_list(theme.get("Inherits"));
        let names = split_list(theme.get("Directories"));
        let directories = names
            .into_iter()
            .map(|name| IconDirectory::from_section(name.clone(), sections.get(&name)))
            .collect();
        Some(Self {
            inherits,
            directories,
        })
    }
}

struct IconDirectory {
    name: String,
    size: u32,
    min_size: u32,
    max_size: u32,
    threshold: u32,
    kind: DirectoryType,
}

#[derive(Clone, Copy)]
enum DirectoryType {
    Fixed,
    Scalable,
    Threshold,
}

impl IconDirectory {
    fn from_section(name: String, section: Option<&HashMap<String, String>>) -> Self {
        let field = |key: &str| section.and_then(|values| values.get(key));
        let size = parse_u32(field("Size")).unwrap_or(48);
        let kind = match field("Type").map(String::as_str) {
            Some("Scalable") => DirectoryType::Scalable,
            Some("Threshold") => DirectoryType::Threshold,
            _ => DirectoryType::Fixed,
        };
        Self {
            name,
            size,
            min_size: parse_u32(field("MinSize")).unwrap_or(size),
            max_size: parse_u32(field("MaxSize")).unwrap_or(size),
            threshold: parse_u32(field("Threshold")).unwrap_or(2),
            kind,
        }
    }

    fn distance(&self, requested: u32) -> u32 {
        let (minimum, maximum) = match self.kind {
            DirectoryType::Fixed => (self.size, self.size),
            DirectoryType::Scalable => (self.min_size, self.max_size),
            DirectoryType::Threshold => (
                self.size.saturating_sub(self.threshold),
                self.size.saturating_add(self.threshold),
            ),
        };
        if requested < minimum {
            minimum - requested
        } else {
            requested.saturating_sub(maximum)
        }
    }
}

fn parse_ini(source: &str) -> HashMap<String, HashMap<String, String>> {
    let mut sections = HashMap::<String, HashMap<String, String>>::new();
    let mut current = String::new();
    for line in source.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            current = section.to_owned();
        } else if let Some((key, value)) = line.split_once('=') {
            sections
                .entry(current.clone())
                .or_default()
                .insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    sections
}

fn split_list(value: Option<&String>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_u32(value: Option<&String>) -> Option<u32> {
    value?.parse().ok()
}

fn find_named_file(directory: &Path, name: &str) -> Option<PathBuf> {
    ["svg", "png", "webp", "jpg", "jpeg"]
        .into_iter()
        .map(|extension| directory.join(format!("{name}.{extension}")))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("mold-image-{name}-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn svg_is_scaled_and_cached_by_physical_size() {
        let root = temp_dir("svg");
        let path = root.join("square.svg");
        fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        let mut cache = ImageCache::default();
        let first = cache.load(&path, 8, 4, 180).unwrap();
        let second = cache.load(&path, 8, 4, 180).unwrap();
        assert_eq!((first.width, first.height), (12, 6));
        assert_eq!(cache.intrinsic_size(&path).unwrap(), (2, 2));
        assert_eq!(&first.rgba[..4], &[255, 0, 0, 255]);
        assert!(Arc::ptr_eq(&first, &second));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn icon_lookup_prefers_closest_directory_and_inherits() {
        let root = temp_dir("icons");
        let parent = root.join("parent");
        let child = root.join("child");
        fs::create_dir_all(parent.join("16x16/apps")).unwrap();
        fs::create_dir_all(parent.join("64x64/apps")).unwrap();
        fs::create_dir_all(&child).unwrap();
        fs::write(
            parent.join("index.theme"),
            "[Icon Theme]\nDirectories=16x16/apps,64x64/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[64x64/apps]\nSize=64\nType=Fixed\n",
        )
        .unwrap();
        fs::write(
            child.join("index.theme"),
            "[Icon Theme]\nInherits=parent\nDirectories=\n",
        )
        .unwrap();
        let expected = parent.join("64x64/apps/demo.svg");
        fs::write(&expected, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        let resolver = IconResolver::new(vec![root.clone()]);
        assert_eq!(resolver.find("demo", "child", 48).unwrap(), expected);
        fs::remove_dir_all(root).unwrap();
    }
}
