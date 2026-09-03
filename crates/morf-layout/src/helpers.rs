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
    if matches!(parent, Element::Row | Element::Grid)
        && (fill || center || flag(anchors, "left") || flag(anchors, "right"))
    {
        return Err(LayoutError::AxisConflict { axis: "horizontal" });
    }
    if matches!(parent, Element::Column | Element::Grid)
        && (fill || center || flag(anchors, "top") || flag(anchors, "bottom"))
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

/// `gap`, or the older `spacing` when `gap` is unset.
pub(crate) fn gap_of(
    scene: &morf_scene::Scene,
    node: morf_scene::NodeHandle,
) -> Result<f64, LayoutError> {
    let gap = scene.number(node, "gap")?;
    Ok(if gap > 0.0 {
        gap
    } else {
        scene.number(node, "spacing")?
    })
}

/// `align`, unless the older `alignment` was set instead.
pub(crate) fn align_of(
    scene: &morf_scene::Scene,
    node: morf_scene::NodeHandle,
) -> Result<String, LayoutError> {
    let alias = scene.string_value(node, "alignment")?;
    Ok(if alias != "start" {
        alias.to_owned()
    } else {
        scene.string_value(node, "align")?.to_owned()
    })
}

/// Where a packed run starts and how much extra goes between children,
/// for `justify`.
pub(crate) fn justify_run(
    justify: &str,
    free: f64,
    count: usize,
) -> Result<(f64, f64), LayoutError> {
    let free = free.max(0.0);
    let gaps = count.saturating_sub(1) as f64;
    Ok(match justify {
        "start" => (0.0, 0.0),
        "center" => (free / 2.0, 0.0),
        "end" => (free, 0.0),
        "space_between" => (0.0, if gaps > 0.0 { free / gaps } else { 0.0 }),
        "space_around" => {
            let each = free / count.max(1) as f64;
            (each / 2.0, each)
        }
        "space_evenly" => {
            let each = free / (count + 1) as f64;
            (each, each)
        }
        other => {
            return Err(LayoutError::Scene(format!(
                "unknown justify `{other}`: use start, center, end, space_between, space_around or space_evenly"
            )));
        }
    })
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
