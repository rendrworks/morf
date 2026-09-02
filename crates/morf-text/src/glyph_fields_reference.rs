//! The field, computed by asking every texel about every segment.
//!
//! Kept as the thing `field_from_segments` is checked against. It is what the
//! generator used to be, and the acceleration is only worth having if the two
//! agree byte for byte. It lives apart from the generator it checks so that
//! neither is read as the other by accident.

use std::rc::Rc;

use crate::glyph_fields::{FieldBox, FieldImage, Segment};

pub(crate) fn field_by_brute_force(
    segments: &[Segment],
    area: FieldBox,
    spread: f32,
) -> Option<FieldImage> {
    if segments.is_empty() {
        return None;
    }
    let left = area.left;
    let top = area.top;
    let width = (area.right - area.left).ceil().max(1.0) as u32;
    let height = (area.top - area.bottom).ceil().max(1.0) as u32;
    let mut data = Vec::with_capacity((width * height) as usize);
    for row in 0..height {
        let y = top - row as f32 - 0.5;
        for column in 0..width {
            let x = left + column as f32 + 0.5;
            let mut nearest = f32::MAX;
            let mut winding = 0;
            for segment in segments {
                nearest = nearest.min(segment.distance_squared(x, y));
                winding += segment.winding(x, y);
            }
            let signed = if winding != 0 {
                -nearest.sqrt()
            } else {
                nearest.sqrt()
            };
            let unit = (signed + spread) / (spread * 2.0);
            data.push((unit.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Some(FieldImage {
        left,
        top,
        width,
        height,
        data: Rc::new(data),
    })
}
