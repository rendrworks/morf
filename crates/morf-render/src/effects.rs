use morf_layout::{Geometry, TextAlignment, TextElide};
use morf_scene::{Color, NodeHandle, Scene, Value};

use crate::{commands::*, damage::*, field::*, sdf::*};

#[derive(Clone, Copy)]
pub(crate) struct LayerConfig {
    pub(crate) enabled: bool,
    pub(crate) blur: f64,
    pub(crate) shadow_color: Color,
    pub(crate) shadow_blur: f64,
    pub(crate) shadow_offset_x: f64,
    pub(crate) shadow_offset_y: f64,
}

pub(crate) fn layer_config(scene: &Scene, node: NodeHandle) -> Result<LayerConfig, RenderError> {
    let Value::Map(layer) = scene.current(node, "layer")? else {
        return Err(RenderError::Scene(
            "layer must be a property map".to_owned(),
        ));
    };
    // An empty table is the default on every node that never asked for a
    // layer, which is nearly all of them; the parsing below has nothing to find.
    if layer.is_empty() {
        return Ok(LayerConfig {
            enabled: false,
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
        });
    }
    let enabled = match layer.get("enabled") {
        None => false,
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => {
            return Err(RenderError::Scene(
                "layer.enabled must be a boolean".to_owned(),
            ));
        }
    };
    let number = |name: &str, non_negative: bool| match layer.get(name) {
        None => Ok(0.0),
        Some(Value::Number(value)) if value.is_finite() && (!non_negative || *value >= 0.0) => {
            Ok(*value)
        }
        Some(_) => Err(RenderError::Scene(format!(
            "layer.{name} must be a {}finite number",
            if non_negative { "non-negative " } else { "" }
        ))),
    };
    let shadow_color = match layer.get("shadow_color") {
        None => Color::rgba8(0, 0, 0, 0),
        Some(Value::Color(color)) => *color,
        Some(_) => {
            return Err(RenderError::Scene(
                "layer.shadow_color must be a color".to_owned(),
            ));
        }
    };
    Ok(LayerConfig {
        enabled,
        blur: number("blur", true)?,
        shadow_color,
        shadow_blur: number("shadow_blur", true)?,
        shadow_offset_x: number("shadow_offset_x", false)?,
        shadow_offset_y: number("shadow_offset_y", false)?,
    })
}

pub(crate) fn command_union(commands: &[DrawCommand]) -> Option<Geometry> {
    commands
        .iter()
        .map(DrawCommand::bounds)
        .reduce(union_geometry)
}

pub(crate) fn union_geometry(left: Geometry, right: Geometry) -> Geometry {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.width).max(right.x + right.width);
    let bottom = (left.y + left.height).max(right.y + right.height);
    Geometry {
        x,
        y,
        width: right_edge - x,
        height: bottom - y,
    }
}

pub(crate) fn expand_geometry(bounds: Geometry, amount: f64) -> Geometry {
    Geometry {
        x: bounds.x - amount,
        y: bounds.y - amount,
        width: bounds.width + amount * 2.0,
        height: bounds.height + amount * 2.0,
    }
}

pub(crate) fn offset_geometry(bounds: Geometry, x: f64, y: f64) -> Geometry {
    Geometry {
        x: bounds.x + x,
        y: bounds.y + y,
        ..bounds
    }
}

pub(crate) fn compose_overlay(under: Color, over: Color) -> Color {
    let alpha = over.alpha + under.alpha * (1.0 - over.alpha);
    if alpha <= f32::EPSILON {
        return Color::rgba8(0, 0, 0, 0);
    }
    Color {
        red: (over.red * over.alpha + under.red * under.alpha * (1.0 - over.alpha)) / alpha,
        green: (over.green * over.alpha + under.green * under.alpha * (1.0 - over.alpha)) / alpha,
        blue: (over.blue * over.alpha + under.blue * under.alpha * (1.0 - over.alpha)) / alpha,
        alpha,
    }
}

pub(crate) fn apply_overlay(color: Color, overlay: Color) -> Color {
    Color {
        red: color.red * (1.0 - overlay.alpha) + overlay.red * overlay.alpha,
        green: color.green * (1.0 - overlay.alpha) + overlay.green * overlay.alpha,
        blue: color.blue * (1.0 - overlay.alpha) + overlay.blue * overlay.alpha,
        alpha: color.alpha,
    }
}

pub(crate) fn intersect_geometry(left: Geometry, right: Geometry) -> Geometry {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    Geometry {
        x,
        y,
        width: (right_edge - x).max(0.0),
        height: (bottom_edge - y).max(0.0),
    }
}

/// Parses where a stroke sits against the edge.
pub(crate) fn stroke_alignment(name: &str) -> Result<BorderAlignment, RenderError> {
    Ok(match name {
        "inside" | "inner" => BorderAlignment::Inside,
        "centre" | "center" | "centred" | "centered" => BorderAlignment::Centred,
        "outside" | "outer" => BorderAlignment::Outside,
        other => {
            return Err(RenderError::Scene(format!(
                "stroke_alignment {other} is not inside, centre or outside"
            )));
        }
    })
}

pub(crate) fn scene_gradient(scene: &Scene, node: NodeHandle) -> Result<Gradient, RenderError> {
    // The kind decides first. Almost every rect in a real configuration has no
    // gradient at all, and reading the two colours before asking made every one
    // of them pay for three property lookups instead of one.
    let kind = scene.string_value(node, "gradient_type")?;
    if kind == "none" {
        return Ok(Gradient::None);
    }
    let start_color = scene.color_value(node, "gradient_start_color")?;
    let end_color = scene.color_value(node, "gradient_end_color")?;
    Ok(match kind {
        "none" => Gradient::None,
        "linear" => Gradient::Linear {
            start_color,
            end_color,
            start: [
                scene.number(node, "gradient_start_x")?,
                scene.number(node, "gradient_start_y")?,
            ],
            end: [
                scene.number(node, "gradient_end_x")?,
                scene.number(node, "gradient_end_y")?,
            ],
        },
        "radial" => {
            let radius = scene.number(node, "gradient_radius")?;
            if radius <= 0.0 {
                return Err(RenderError::Scene(
                    "Rect radial gradient radius must be positive".to_owned(),
                ));
            }
            Gradient::Radial {
                start_color,
                end_color,
                center: [
                    scene.number(node, "gradient_center_x")?,
                    scene.number(node, "gradient_center_y")?,
                ],
                radius,
            }
        }
        "conical" => Gradient::Conical {
            start_color,
            end_color,
            center: [
                scene.number(node, "gradient_center_x")?,
                scene.number(node, "gradient_center_y")?,
            ],
            angle: scene.number(node, "gradient_angle")?,
        },
        kind => {
            return Err(RenderError::Scene(format!(
                "unknown Rect gradient type `{kind}`"
            )));
        }
    })
}

pub(crate) fn rect_radii(scene: &Scene, node: NodeHandle) -> Result<[f64; 4], RenderError> {
    let uniform = scene.number(node, "radius")?.max(0.0);
    let corner = |property| -> Result<f64, RenderError> {
        let value = scene.number(node, property)?;
        Ok(if value < 0.0 { uniform } else { value })
    };
    Ok([
        corner("top_left_radius")?,
        corner("top_right_radius")?,
        corner("bottom_right_radius")?,
        corner("bottom_left_radius")?,
    ])
}

pub(crate) fn render_text_alignment(value: &str) -> Result<TextAlignment, RenderError> {
    match value {
        "left" => Ok(TextAlignment::Left),
        "right" => Ok(TextAlignment::Right),
        "center" => Ok(TextAlignment::Center),
        "justified" => Ok(TextAlignment::Justified),
        _ => Err(RenderError::Scene(format!(
            "unknown Text horizontal alignment `{value}`"
        ))),
    }
}

pub(crate) fn render_text_elide(value: &str) -> Result<TextElide, RenderError> {
    match value {
        "none" => Ok(TextElide::None),
        "left" => Ok(TextElide::Left),
        "middle" => Ok(TextElide::Middle),
        "right" => Ok(TextElide::Right),
        _ => Err(RenderError::Scene(format!(
            "unknown text elide mode `{value}`"
        ))),
    }
}

pub(crate) fn vertical_alignment(value: &str) -> Result<VerticalAlignment, RenderError> {
    match value {
        "top" => Ok(VerticalAlignment::Top),
        "center" => Ok(VerticalAlignment::Center),
        "bottom" => Ok(VerticalAlignment::Bottom),
        _ => Err(RenderError::Scene(format!(
            "unknown Text vertical alignment `{value}`"
        ))),
    }
}

pub(crate) fn image_fill_mode(value: &str) -> Result<ImageFillMode, RenderError> {
    match value {
        "stretch" => Ok(ImageFillMode::Stretch),
        "preserve_aspect_fit" => Ok(ImageFillMode::PreserveAspectFit),
        "preserve_aspect_crop" => Ok(ImageFillMode::PreserveAspectCrop),
        _ => Err(RenderError::Scene(format!(
            "unknown image fill mode `{value}`"
        ))),
    }
}

pub(crate) fn gradient_instance(
    gradient: &Gradient,
) -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4], f32) {
    match gradient {
        Gradient::None => ([0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4], 0.0),
        Gradient::Linear {
            start_color,
            end_color,
            start,
            end,
        } => (
            color_array(*start_color),
            color_array(*end_color),
            [
                start[0] as f32,
                start[1] as f32,
                end[0] as f32,
                end[1] as f32,
            ],
            [1.0, 0.0, 0.0, 0.0],
            0.0,
        ),
        Gradient::Radial {
            start_color,
            end_color,
            center,
            radius,
        } => (
            color_array(*start_color),
            color_array(*end_color),
            [0.0; 4],
            [2.0, center[0] as f32, center[1] as f32, *radius as f32],
            0.0,
        ),
        Gradient::Conical {
            start_color,
            end_color,
            center,
            angle,
        } => (
            color_array(*start_color),
            color_array(*end_color),
            [0.0; 4],
            [3.0, center[0] as f32, center[1] as f32, 0.0],
            angle.to_radians() as f32,
        ),
    }
}

pub(crate) fn effect_bounds(
    bounds: Geometry,
    blur: f64,
    shadow_blur: f64,
    shadow_spread: f64,
    shadow_offset_x: f64,
    shadow_offset_y: f64,
) -> Geometry {
    let blur = blur.max(0.0);
    let shadow = (shadow_blur.max(0.0) + shadow_spread.max(0.0)).max(0.0);
    let left = blur.max(shadow - shadow_offset_x);
    let right = blur.max(shadow + shadow_offset_x);
    let top = blur.max(shadow - shadow_offset_y);
    let bottom = blur.max(shadow + shadow_offset_y);
    Geometry {
        x: bounds.x - left,
        y: bounds.y - top,
        width: bounds.width + left + right,
        height: bounds.height + top + bottom,
    }
}

pub(crate) fn color_array(color: Color) -> [f32; 4] {
    [
        srgb_channel_to_linear(color.red),
        srgb_channel_to_linear(color.green),
        srgb_channel_to_linear(color.blue),
        color.alpha,
    ]
}

pub(crate) fn srgb_channel_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn physical_damage(geometry: Geometry, scale_120: u32) -> Option<DamageRect> {
    if geometry.width <= 0.0 || geometry.height <= 0.0 {
        return None;
    }
    let scale = scale_120.max(1) as f64 / 120.0;
    let left = (geometry.x * scale).floor().max(0.0) as u32;
    let top = (geometry.y * scale).floor().max(0.0) as u32;
    let right = ((geometry.x + geometry.width) * scale).ceil().max(0.0) as u32;
    let bottom = ((geometry.y + geometry.height) * scale).ceil().max(0.0) as u32;
    Some(DamageRect {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    })
}

/// Collapses overlapping and touching damage into as few rectangles as it can.
///
/// Absorbing one rectangle grows another, and a grown rectangle can reach ones
/// it did not touch a moment ago, so the scan restarts after every merge. What
/// it must not also do is shift the whole tail down to close the gap:
/// `Vec::remove` is linear, and it was being paid once per merge on the
/// per-frame damage path. Damage rectangles are an unordered set — the
/// compositor is told about each one independently — so the last element can
/// simply be moved into the hole instead.
pub(crate) fn merge_damage(mut damage: Vec<DamageRect>) -> Vec<DamageRect> {
    let mut index = 0;
    while index < damage.len() {
        let mut other = index + 1;
        while other < damage.len() {
            if touches(damage[index], damage[other]) {
                damage[index] = union(damage[index], damage.swap_remove(other));
                other = index + 1;
            } else {
                other += 1;
            }
        }
        index += 1;
    }
    damage
}

fn touches(left: DamageRect, right: DamageRect) -> bool {
    left.x <= right.x.saturating_add(right.width)
        && right.x <= left.x.saturating_add(left.width)
        && left.y <= right.y.saturating_add(right.height)
        && right.y <= left.y.saturating_add(left.height)
}

fn union(left: DamageRect, right: DamageRect) -> DamageRect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = left
        .x
        .saturating_add(left.width)
        .max(right.x.saturating_add(right.width));
    let bottom = left
        .y
        .saturating_add(left.height)
        .max(right.y.saturating_add(right.height));
    DamageRect {
        x,
        y,
        width: right_edge - x,
        height: bottom - y,
    }
}
