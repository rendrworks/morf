// Glyphs as distance fields rather than as coverage.
//
// A coverage bitmap is a picture of a glyph at one size, with one subpixel
// offset, and it is worth nothing at any other size: the atlas fills up with
// the same letter over and over as a configuration animates a font size or a
// display changes scale. A distance field is a picture of the *shape*. One
// entry per glyph serves every size, and because the edge is a threshold
// rather than a set of pixels, an outline is a second threshold and a heavier
// weight is the first one moved — both of them ordinary animatable numbers
// instead of a re-render.

use std::rc::Rc;

use signed_distance_field::binary_image;
use signed_distance_field::compute_f32_distance_field;
use signed_distance_field::distance_field::DistanceStorage;

/// The size every glyph is rasterized at before its field is measured.
///
/// Large enough that the field records the shape rather than the rasterizer's
/// opinion of it at some particular size, small enough that a page of text is
/// a handful of atlas entries. Text is drawn at whatever size it likes by
/// scaling the quad; nothing is rasterized again.
pub const FIELD_REFERENCE_PX: f32 = 64.0;

/// How far outside the glyph the field is measured, in reference pixels.
///
/// This is the room an outline has to live in, and the distance over which the
/// edge can be moved to thicken or thin the letter. Everything beyond it reads
/// as "far outside" and cannot be drawn into.
pub const FIELD_SPREAD_PX: u32 = 8;

/// Turns one rasterized coverage bitmap into a distance field of the same
/// shape, padded so the field has somewhere to go outside the glyph.
///
/// The bytes run from zero at the outside of the spread to one at the inside,
/// which is the direction the shader thresholds in: below the edge is ink.
/// Returns `None` for a glyph with no ink at all — a space has no shape to
/// measure, and the distance transform has nothing to work from.
pub(crate) fn glyph_field(alpha: &[u8], width: u32, height: u32) -> Option<FieldImage> {
    let pad = FIELD_SPREAD_PX;
    let padded_width = width + pad * 2;
    let padded_height = height + pad * 2;
    let mut padded = vec![0u8; (padded_width * padded_height) as usize];
    for row in 0..height {
        let source = (row * width) as usize;
        let target = ((row + pad) * padded_width + pad) as usize;
        padded[target..target + width as usize]
            .copy_from_slice(&alpha[source..source + width as usize]);
    }

    let binary = binary_image::of_byte_slice(
        &padded,
        u16::try_from(padded_width).ok()?,
        u16::try_from(padded_height).ok()?,
    );
    let spread = FIELD_SPREAD_PX as f32;
    let field = compute_f32_distance_field(&binary).normalize_clamped_distances(-spread, spread)?;
    let data = Rc::new(
        (0..padded.len())
            .map(|index| (field.distances.get(index).clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect(),
    );
    Some(FieldImage {
        left: 0,
        top: 0,
        width: padded_width,
        height: padded_height,
        data,
    })
}

/// One glyph's field, and where it sits relative to the pen.
///
/// `left` and `top` are the reference-size placement of the *padded* box, so
/// scaling them by the size being drawn gives the quad directly.
pub(crate) struct FieldImage {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) data: Rc<Vec<u8>>,
}
