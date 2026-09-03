use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;

use morf_scene::{Element, SceneError, Value};

use crate::geometry::{Geometry, Size, TextAlignment, TextElide};

/// A layout input or constraint failure.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutError {
    /// The scene graph rejected a read.
    Scene(String),
    /// The anchors property was not a string-keyed table.
    InvalidAnchors,
    /// An inset margin was neither nil nor a finite number.
    InvalidInsetMargin(&'static str),
    /// The supplied transform common parent is not an ancestor of both nodes.
    InvalidCommonParent,
    /// Anchors and a positioner both control the same axis.
    AxisConflict { axis: &'static str },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(message) => write!(f, "scene layout error: {message}"),
            Self::InvalidAnchors => f.write_str("anchors must be a string-keyed map"),
            Self::InvalidInsetMargin(property) => {
                write!(f, "{property} must be nil or a finite number")
            }
            Self::InvalidCommonParent => {
                f.write_str("transform common parent must contain both nodes")
            }
            Self::AxisConflict { axis: "flex" } => f.write_str(
                "anchors inside a Flex or track Grid: the container places its children",
            ),
            Self::AxisConflict { axis } => {
                write!(f, "anchors and positioner both control the {axis} axis")
            }
        }
    }
}

impl StdError for LayoutError {}

impl From<SceneError> for LayoutError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error.to_string())
    }
}

pub(crate) fn positive(value: f64) -> Option<f64> {
    (value > 0.0).then_some(value)
}

pub(crate) fn text_alignment(value: &str) -> Result<TextAlignment, LayoutError> {
    match value {
        "left" => Ok(TextAlignment::Left),
        "right" => Ok(TextAlignment::Right),
        "center" => Ok(TextAlignment::Center),
        "justified" => Ok(TextAlignment::Justified),
        _ => Err(LayoutError::Scene(format!(
            "unknown Text horizontal alignment `{value}`"
        ))),
    }
}

pub(crate) fn text_elide(value: &str) -> Result<TextElide, LayoutError> {
    match value {
        "none" => Ok(TextElide::None),
        "left" => Ok(TextElide::Left),
        "middle" => Ok(TextElide::Middle),
        "right" => Ok(TextElide::Right),
        _ => Err(LayoutError::Scene(format!(
            "unknown text elide mode `{value}`"
        ))),
    }
}

pub(crate) fn sum_with_spacing(children: &[Size], spacing: f64, horizontal: bool) -> f64 {
    let content = children
        .iter()
        .map(|size| if horizontal { size.width } else { size.height })
        .sum::<f64>();
    content + spacing * children.len().saturating_sub(1) as f64
}

pub(crate) fn grid_columns(value: f64) -> usize {
    if value.is_finite() && value >= 1.0 {
        value.floor().min(usize::MAX as f64) as usize
    } else {
        1
    }
}

pub(crate) fn grid_size(
    children: &[Size],
    columns: usize,
    column_spacing: f64,
    row_spacing: f64,
) -> Size {
    if children.is_empty() {
        return Size::default();
    }
    let rows = children.len().div_ceil(columns);
    let mut widths = vec![0.0_f64; columns];
    let mut heights = vec![0.0_f64; rows];
    for (index, child) in children.iter().enumerate() {
        widths[index % columns] = widths[index % columns].max(child.width);
        heights[index / columns] = heights[index / columns].max(child.height);
    }
    Size {
        width: widths.into_iter().sum::<f64>() + column_spacing * columns.saturating_sub(1) as f64,
        height: heights.into_iter().sum::<f64>() + row_spacing * rows.saturating_sub(1) as f64,
    }
}

/// Borrows a node's attached layout constraints.
///
/// Borrowed rather than cloned: layout asks for these once per child per pass,
/// and every clone copied a `BTreeMap` of owned strings and values for a table
/// that is usually empty and never modified here.
pub(crate) fn attached_layout(value: &Value) -> Result<&BTreeMap<String, Value>, LayoutError> {
    match value {
        Value::Map(map) => Ok(map),
        _ => Err(LayoutError::Scene(
            "attached layout constraints must be a map".to_owned(),
        )),
    }
}

pub(crate) fn layout_string<'a>(map: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    match map.get(key) {
        Some(Value::String(value)) => Some(value.as_str()),
        _ => None,
    }
}

/// A weight from the attached layout map: finite, non-negative, else `default`.
pub(crate) fn layout_weight(map: &BTreeMap<String, Value>, key: &str, default: f64) -> f64 {
    layout_number(map, key).unwrap_or(default)
}

pub(crate) fn layout_number(map: &BTreeMap<String, Value>, key: &str) -> Option<f64> {
    number(map, key).filter(|value| value.is_finite() && *value >= 0.0)
}

pub(crate) fn clamp_layout(
    value: f64,
    map: &BTreeMap<String, Value>,
    minimum: &str,
    maximum: &str,
) -> f64 {
    let minimum = layout_number(map, minimum).unwrap_or(0.0);
    let maximum = layout_number(map, maximum).unwrap_or(f64::INFINITY);
    value.max(minimum).min(maximum.max(minimum))
}

/// Borrows a node's anchors, for the same reason as [`attached_layout`].
pub(crate) fn anchors(value: &Value) -> Result<&BTreeMap<String, Value>, LayoutError> {
    match value {
        Value::Map(map) => Ok(map),
        _ => Err(LayoutError::InvalidAnchors),
    }
}

pub(crate) fn reject_axis_conflict(
    parent: Element,
    anchors: &BTreeMap<String, Value>,
) -> Result<(), LayoutError> {
    let fill = flag(anchors, "fill");
    let center = flag(anchors, "center_in");
    if matches!(
        parent,
        Element::Row | Element::RowLayout | Element::Grid | Element::GridLayout
    ) && (fill || center || flag(anchors, "left") || flag(anchors, "right"))
    {
        return Err(LayoutError::AxisConflict { axis: "horizontal" });
    }
    if matches!(
        parent,
        Element::Column | Element::ColumnLayout | Element::Grid | Element::GridLayout
    ) && (fill || center || flag(anchors, "top") || flag(anchors, "bottom"))
    {
        return Err(LayoutError::AxisConflict { axis: "vertical" });
    }
    Ok(())
}

pub(crate) fn apply_anchors(
    parent: Geometry,
    anchors: &BTreeMap<String, Value>,
    geometry: &mut Geometry,
) {
    let margin = number(anchors, "margins").unwrap_or(0.0);
    let left_margin = number(anchors, "left_margin").unwrap_or(margin);
    let right_margin = number(anchors, "right_margin").unwrap_or(margin);
    let top_margin = number(anchors, "top_margin").unwrap_or(margin);
    let bottom_margin = number(anchors, "bottom_margin").unwrap_or(margin);
    if flag(anchors, "fill") {
        geometry.x = left_margin;
        geometry.y = top_margin;
        geometry.width = (parent.width - left_margin - right_margin).max(0.0);
        geometry.height = (parent.height - top_margin - bottom_margin).max(0.0);
        return;
    }
    if flag(anchors, "center_in") {
        geometry.x = (parent.width - geometry.width) / 2.0;
        geometry.y = (parent.height - geometry.height) / 2.0;
    }
    if flag(anchors, "left") {
        geometry.x = left_margin;
        if flag(anchors, "right") {
            geometry.width = (parent.width - left_margin - right_margin).max(0.0);
        }
    } else if flag(anchors, "right") {
        geometry.x = parent.width - geometry.width - right_margin;
    }
    if flag(anchors, "top") {
        geometry.y = top_margin;
        if flag(anchors, "bottom") {
            geometry.height = (parent.height - top_margin - bottom_margin).max(0.0);
        }
    } else if flag(anchors, "bottom") {
        geometry.y = parent.height - geometry.height - bottom_margin;
    }
}

pub(crate) fn flag(map: &BTreeMap<String, Value>, key: &str) -> bool {
    matches!(map.get(key), Some(Value::Bool(true)))
}

fn number(map: &BTreeMap<String, Value>, key: &str) -> Option<f64> {
    match map.get(key) {
        Some(Value::Number(value)) => Some(*value),
        _ => None,
    }
}
