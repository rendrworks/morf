use signed_distance_field::binary_image;
use signed_distance_field::distance_field::DistanceStorage;
use signed_distance_field::compute_f32_distance_field;

fn distance_field_from_alpha(
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
