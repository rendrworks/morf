use signed_distance_field::binary_image;
use signed_distance_field::compute_f32_distance_field;
use signed_distance_field::distance_field::DistanceStorage;

use crate::image_cache::ImageError;
use crate::quantize::ImageData;

/// Measures a distance field from an image's alpha, in place.
///
/// Unlike the glyph fields in `mold-text`, this does **not** pad the source
/// first, so the field saturates at the image border and an outline asked for
/// near an edge is cut off by it. Padding here would change the image's
/// dimensions, and the texture quad is the node's own rectangle — the glyph
/// path can pad because it sizes its quad from the field, and this one cannot
/// without that same change. Sources with their own transparent margin, which
/// is most SVG icons, are unaffected.
pub(crate) fn distance_field_from_alpha(
    image: &ImageData,
    spread: f32,
) -> Result<ImageData, ImageError> {
    let width = u16::try_from(image.width).map_err(|_| ImageError::InvalidSize)?;
    let height = u16::try_from(image.height).map_err(|_| ImageError::InvalidSize)?;
    let alpha = image
        .rgba
        .chunks_exact(4)
        .map(|pixel| pixel[3])
        .collect::<Vec<_>>();
    let binary = binary_image::of_byte_slice(&alpha, width, height);
    let spread = spread.max(0.5);
    let field = compute_f32_distance_field(&binary)
        .normalize_clamped_distances(-spread, spread)
        .ok_or(ImageError::DistanceFieldEmpty)?;
    let mut rgba = Vec::with_capacity(alpha.len() * 4);
    for index in 0..alpha.len() {
        let distance = (field.distances.get(index).clamp(0.0, 1.0) * 255.0).round() as u8;
        rgba.extend_from_slice(&[distance, distance, distance, 255]);
    }
    Ok(ImageData {
        width: image.width,
        height: image.height,
        rgba,
    })
}
