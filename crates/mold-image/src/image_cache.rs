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
    /// The source URI did not identify a local file.
    InvalidSource(String),
    /// The source alpha mask had no detectable boundary.
    DistanceFieldEmpty,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "image I/O failed: {error}"),
            Self::Raster(error) => write!(f, "image decode failed: {error}"),
            Self::Svg(error) => write!(f, "SVG decode failed: {error}"),
            Self::IconNotFound(name) => write!(f, "icon `{name}` was not found"),
            Self::InvalidSize => f.write_str("image size must be greater than zero"),
            Self::InvalidSource(source) => write!(f, "invalid local image source `{source}`"),
            Self::DistanceFieldEmpty => f.write_str("distance-field source has no alpha edge"),
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DistanceFieldKey {
    image: CacheKey,
    spread: u32,
}

/// Cache for decoded images and resolved icon names.
#[derive(Default)]
pub struct ImageCache {
    images: HashMap<CacheKey, Arc<ImageData>>,
    icons: HashMap<(String, String, u32, u32), PathBuf>,
    intrinsic: HashMap<PathBuf, (u32, u32)>,
    distance_fields: HashMap<DistanceFieldKey, Arc<ImageData>>,
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
        let source = normalize_source(source.as_ref())?;
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

    /// Loads an image alpha mask and caches its normalized signed distance field.
    pub fn load_distance_field(
        &mut self,
        source: impl AsRef<Path>,
        logical_width: u32,
        logical_height: u32,
        scale_120: u32,
        spread: f32,
    ) -> Result<Arc<ImageData>, ImageError> {
        let source = normalize_source(source.as_ref())?;
        let width = physical_size(logical_width, scale_120)?;
        let height = physical_size(logical_height, scale_120)?;
        let key = DistanceFieldKey {
            image: CacheKey {
                source: source.clone(),
                width,
                height,
                scale_120,
            },
            spread: spread.max(0.5).to_bits(),
        };
        if let Some(image) = self.distance_fields.get(&key) {
            return Ok(Arc::clone(image));
        }
        let image = self.load(source, logical_width, logical_height, scale_120)?;
        let field = Arc::new(distance_field_from_alpha(&image, spread)?);
        self.distance_fields.insert(key, Arc::clone(&field));
        Ok(field)
    }

    /// Resolves an icon and caches a signed distance field from its alpha mask.
    pub fn load_icon_distance_field_sized(
        &mut self,
        name: &str,
        theme: &str,
        logical_width: u32,
        logical_height: u32,
        scale_120: u32,
        spread: f32,
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
        self.load_distance_field(
            path,
            logical_width,
            logical_height,
            scale_120,
            spread,
        )
    }

    /// Returns a source's unscaled pixel dimensions.
    pub fn intrinsic_size(&mut self, source: impl AsRef<Path>) -> Result<(u32, u32), ImageError> {
        let source = normalize_source(source.as_ref())?;
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
        self.distance_fields.clear();
    }
}

fn physical_size(logical: u32, scale_120: u32) -> Result<u32, ImageError> {
    if logical == 0 || scale_120 == 0 {
        return Err(ImageError::InvalidSize);
    }
    Ok(logical.saturating_mul(scale_120).div_ceil(120))
}

fn normalize_source(source: &Path) -> Result<PathBuf, ImageError> {
    let Some(value) = source.to_str() else {
        return Ok(source.to_path_buf());
    };
    let Some(uri) = value.strip_prefix("file://") else {
        return Ok(source.to_path_buf());
    };
    let uri = uri.strip_prefix("localhost").unwrap_or(uri);
    if !uri.starts_with('/') {
        return Err(ImageError::InvalidSource(value.to_owned()));
    }
    let bytes = uri.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(high) = bytes.get(index + 1).and_then(|value| hex_digit(*value)) else {
                return Err(ImageError::InvalidSource(value.to_owned()));
            };
            let Some(low) = bytes.get(index + 2).and_then(|value| hex_digit(*value)) else {
                return Err(ImageError::InvalidSource(value.to_owned()));
            };
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Ok(std::ffi::OsString::from_vec(decoded).into())
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
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
