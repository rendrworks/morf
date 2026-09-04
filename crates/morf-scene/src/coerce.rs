//! What a property accepts, and the value it keeps.
//!
//! Every write goes through here: a type is checked, a colour string
//! becomes a colour, and the few properties with a shape of their own — a
//! gradient, a decoration, a line height, a cursor — are refused where they
//! are written rather than where they are painted.

use crate::{animation::*, decoration::TextDecoration, gradient::Gradient, types::*};

/// Every shape a `MouseArea` may ask the pointer to take: the cursor-shape
/// protocol's own list, spelled with underscores.
pub const CURSOR_SHAPES: [&str; 36] = [
    "default",
    "context_menu",
    "help",
    "pointer",
    "progress",
    "wait",
    "cell",
    "crosshair",
    "text",
    "vertical_text",
    "alias",
    "copy",
    "move",
    "no_drop",
    "not_allowed",
    "grab",
    "grabbing",
    "e_resize",
    "n_resize",
    "ne_resize",
    "nw_resize",
    "s_resize",
    "se_resize",
    "sw_resize",
    "w_resize",
    "ew_resize",
    "ns_resize",
    "nesw_resize",
    "nwse_resize",
    "col_resize",
    "row_resize",
    "all_scroll",
    "zoom_in",
    "zoom_out",
    "dnd_ask",
    "all_resize",
];

pub(crate) fn coerce(
    element: Element,
    property: &str,
    kind: PropertyType,
    value: Value,
) -> Result<Value, SceneError> {
    let invalid = |message: String| SceneError::InvalidPropertyValue {
        element: element.name(),
        property: property.to_owned(),
        message,
    };
    match property {
        "gradient" => return Gradient::canonical(value).map_err(invalid),
        "decoration" => return TextDecoration::canonical(value).map_err(invalid),
        "line_height" => {
            // A bare number is a multiple of the font size; a `px` string is
            // a size. Checked here so a wrong one is refused where written.
            return match &value {
                Value::Number(multiple) if multiple.is_finite() && *multiple > 0.0 => Ok(value),
                Value::String(text)
                    if text
                        .strip_suffix("px")
                        .and_then(|pixels| pixels.trim().parse::<f64>().ok())
                        .is_some_and(|pixels| pixels.is_finite() && pixels > 0.0) =>
                {
                    Ok(value)
                }
                _ => Err(invalid(
                    "a multiple of the font size or a `px` size".to_owned(),
                )),
            };
        }
        "cursor" => {
            if let Value::String(name) = &value
                && !CURSOR_SHAPES.contains(&name.as_str())
            {
                return Err(invalid(format!("`{name}` is not a cursor shape")));
            }
        }
        "font_style" => {
            if let Value::String(name) = &value
                && !matches!(name.as_str(), "normal" | "italic" | "oblique")
            {
                return Err(invalid(format!(
                    "`{name}` is not normal, italic or oblique"
                )));
            }
        }
        "font_stretch" => {
            if let Value::String(name) = &value
                && !matches!(
                    name.as_str(),
                    "ultra_condensed"
                        | "extra_condensed"
                        | "condensed"
                        | "semi_condensed"
                        | "normal"
                        | "semi_expanded"
                        | "expanded"
                        | "extra_expanded"
                        | "ultra_expanded"
                )
            {
                return Err(invalid(format!(
                    "`{name}` is not a width from ultra_condensed to ultra_expanded"
                )));
            }
        }
        _ => {}
    }
    // `color` is the one property that may say "inherit": text takes the
    // nearest ancestor's colour, and an `Item` carries one for it without
    // painting anything.
    let converted = match (kind, value) {
        (_, Value::String(value)) if property == "color" && value == "inherit" => {
            Some(Value::String(value))
        }
        (PropertyType::Any, Value::String(value)) if property == "color" => {
            Color::parse(&value).map(Value::Color)
        }
        (PropertyType::Any, value @ (Value::Nil | Value::Color(_))) if property == "color" => {
            Some(value)
        }
        (PropertyType::Any, _) if property == "color" => None,
        (PropertyType::Any, value) => Some(value),
        (PropertyType::Bool, Value::Bool(value)) => Some(Value::Bool(value)),
        (PropertyType::Number, Value::Number(value)) if value.is_finite() => {
            Some(Value::Number(value))
        }
        (PropertyType::String, Value::String(value)) => Some(Value::String(value)),
        (PropertyType::Color, Value::Color(value)) => Some(Value::Color(value)),
        (PropertyType::Color, Value::String(value)) => Color::parse(&value).map(Value::Color),
        _ => None,
    };
    converted.ok_or_else(|| SceneError::InvalidPropertyType {
        element: element.name(),
        property: property.to_owned(),
        expected: match kind {
            PropertyType::Any if property == "color" => "color, inherit or nil",
            PropertyType::Any => "value",
            PropertyType::Bool => "boolean",
            PropertyType::Number => "finite number",
            PropertyType::String => "string",
            PropertyType::Color => "color",
        },
    })
}
