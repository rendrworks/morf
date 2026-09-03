//! Lua-shaped layout values into Taffy styles.
//!
//! A configuration writes `gap = 8`, `align = "center"`, `layout = { grow =
//! 1, width = "50%" }`, `template_columns = { "1fr", "auto" }`; Taffy wants
//! its own structs. This is the translation, and every unknown word is an
//! error at layout time that names itself.

use std::collections::BTreeMap;

use morf_scene::{Element, NodeHandle, Scene, Value};
use taffy::prelude::*;
use taffy::style::{
    AlignContent, AlignItems, GridTemplateComponent, GridTemplateRepetition,
    MaxTrackSizingFunction, MinTrackSizingFunction, RepetitionCount, TrackSizingFunction,
};

use crate::attached::{Attached, Bound};
use crate::helpers::{LayoutError, align_of, attached_layout, gap_of, positive};

fn bad(what: &str, value: &str) -> LayoutError {
    LayoutError::Scene(format!("unknown {what} `{value}`"))
}

/// A size word: a number of pixels, `"50%"`, `"auto"`, `"min_content"`,
/// `"max_content"`.
fn dimension(value: &Value, what: &str) -> Result<Dimension, LayoutError> {
    match value {
        Value::Number(pixels) if pixels.is_finite() => Ok(Dimension::length(*pixels as f32)),
        Value::String(word) => match word.as_str() {
            "auto" => Ok(Dimension::auto()),
            "min_content" => Ok(Dimension::min_content()),
            "max_content" => Ok(Dimension::max_content()),
            other => percent_of(other)
                .map(Dimension::percent)
                .ok_or_else(|| bad(what, other)),
        },
        _ => Err(bad(what, &format!("{value:?}"))),
    }
}

fn percent_of(word: &str) -> Option<f32> {
    word.strip_suffix('%')
        .and_then(|number| number.trim().parse::<f32>().ok())
        .map(|percent| percent / 100.0)
}

fn length_percentage_auto(value: &Value, what: &str) -> Result<LengthPercentageAuto, LayoutError> {
    match value {
        Value::Number(pixels) if pixels.is_finite() => {
            Ok(LengthPercentageAuto::length(*pixels as f32))
        }
        Value::String(word) if word == "auto" => Ok(LengthPercentageAuto::auto()),
        Value::String(word) => percent_of(word)
            .map(LengthPercentageAuto::percent)
            .ok_or_else(|| bad(what, word)),
        _ => Err(bad(what, &format!("{value:?}"))),
    }
}

fn align_items(word: &str) -> Result<Option<AlignItems>, LayoutError> {
    Ok(Some(match word {
        "start" => AlignItems::START,
        "end" => AlignItems::END,
        "center" => AlignItems::CENTER,
        "stretch" => AlignItems::STRETCH,
        "baseline" => AlignItems::BASELINE,
        "auto" => return Ok(None),
        other => return Err(bad("alignment", other)),
    }))
}

fn align_content(word: &str) -> Result<Option<AlignContent>, LayoutError> {
    Ok(Some(match word {
        "start" => AlignContent::START,
        "end" => AlignContent::END,
        "center" => AlignContent::CENTER,
        "stretch" => AlignContent::STRETCH,
        "space_between" => AlignContent::SPACE_BETWEEN,
        "space_around" => AlignContent::SPACE_AROUND,
        "space_evenly" => AlignContent::SPACE_EVENLY,
        "auto" => return Ok(None),
        other => return Err(bad("justification", other)),
    }))
}

/// One track: `"1fr"`, `"auto"`, a number, `"min_content"`, `"max_content"`,
/// or `{ min = ..., max = ... }`.
fn track(value: &Value) -> Result<TrackSizingFunction, LayoutError> {
    fn min_of(value: &Value) -> Result<MinTrackSizingFunction, LayoutError> {
        match value {
            Value::Number(pixels) => Ok(MinTrackSizingFunction::length(*pixels as f32)),
            Value::String(word) => match word.as_str() {
                "auto" => Ok(MinTrackSizingFunction::auto()),
                "min_content" => Ok(MinTrackSizingFunction::min_content()),
                "max_content" => Ok(MinTrackSizingFunction::max_content()),
                other => percent_of(other)
                    .map(MinTrackSizingFunction::percent)
                    .ok_or_else(|| bad("track minimum", other)),
            },
            _ => Err(bad("track minimum", &format!("{value:?}"))),
        }
    }
    fn max_of(value: &Value) -> Result<MaxTrackSizingFunction, LayoutError> {
        match value {
            Value::Number(pixels) => Ok(MaxTrackSizingFunction::length(*pixels as f32)),
            Value::String(word) => match word.as_str() {
                "auto" => Ok(MaxTrackSizingFunction::auto()),
                "min_content" => Ok(MaxTrackSizingFunction::min_content()),
                "max_content" => Ok(MaxTrackSizingFunction::max_content()),
                other => {
                    if let Some(fraction) = other.strip_suffix("fr") {
                        return fraction
                            .trim()
                            .parse::<f32>()
                            .map(MaxTrackSizingFunction::fr)
                            .map_err(|_| bad("track", other));
                    }
                    percent_of(other)
                        .map(MaxTrackSizingFunction::percent)
                        .ok_or_else(|| bad("track", other))
                }
            },
            _ => Err(bad("track", &format!("{value:?}"))),
        }
    }
    match value {
        Value::Map(bounds) => Ok(TrackSizingFunction {
            min: min_of(bounds.get("min").unwrap_or(&Value::String("auto".into())))?,
            max: max_of(bounds.get("max").unwrap_or(&Value::String("auto".into())))?,
        }),
        Value::String(word) if word.ends_with("fr") => Ok(TrackSizingFunction {
            min: MinTrackSizingFunction::auto(),
            max: max_of(value)?,
        }),
        other => Ok(TrackSizingFunction {
            min: min_of(other)?,
            max: max_of(other)?,
        }),
    }
}

/// A track list, with `"repeat(n, ...)"` entries expanded by Taffy.
fn tracks(value: &Value) -> Result<Vec<GridTemplateComponent<String>>, LayoutError> {
    let Value::List(items) = value else {
        return Err(bad("track list", &format!("{value:?}")));
    };
    let mut components = Vec::with_capacity(items.len());
    for item in items {
        if let Value::String(word) = item
            && let Some(inner) = word
                .strip_prefix("repeat(")
                .and_then(|rest| rest.strip_suffix(')'))
        {
            let (count, rest) = inner.split_once(',').ok_or_else(|| bad("repeat", word))?;
            let count = match count.trim() {
                "auto_fill" => RepetitionCount::AutoFill,
                "auto_fit" => RepetitionCount::AutoFit,
                number => RepetitionCount::Count(
                    number
                        .parse::<u16>()
                        .map_err(|_| bad("repeat count", number))?,
                ),
            };
            let repeated = rest
                .split_whitespace()
                .map(|word| track(&Value::String(word.to_owned())))
                .collect::<Result<Vec<_>, _>>()?;
            components.push(GridTemplateComponent::Repeat(GridTemplateRepetition {
                count,
                tracks: repeated,
                line_names: Vec::new(),
            }));
            continue;
        }
        components.push(GridTemplateComponent::Single(track(item)?));
    }
    Ok(components)
}

/// `layout.column = 2`, `layout.column = { 1, 3 }`, plus `layout.column_span`.
fn placement(
    attached: &BTreeMap<String, Value>,
    axis: &str,
) -> Result<Line<GridPlacement<String>>, LayoutError> {
    let mut line = Line {
        start: GridPlacement::Auto,
        end: GridPlacement::Auto,
    };
    match attached.get(axis) {
        None | Some(Value::Nil) => {}
        Some(Value::Number(index)) => {
            line.start = GridPlacement::from_line_index(*index as i16);
        }
        Some(Value::List(range)) if range.len() == 2 => {
            if let (Value::Number(start), Value::Number(end)) = (&range[0], &range[1]) {
                line.start = GridPlacement::from_line_index(*start as i16);
                line.end = GridPlacement::from_line_index(*end as i16);
            } else {
                return Err(bad(axis, &format!("{range:?}")));
            }
        }
        Some(other) => return Err(bad(axis, &format!("{other:?}"))),
    }
    if let Some(Value::Number(span)) = attached.get(&format!("{axis}_span"))
        && *span >= 1.0
    {
        line.end = GridPlacement::from_span(*span as u16);
    }
    Ok(line)
}

/// Whether a node is laid out by Taffy: a `Flex`, or a `Grid` with tracks.
pub(crate) fn is_flex_root(scene: &Scene, node: NodeHandle) -> Result<bool, LayoutError> {
    Ok(match scene.element(node)? {
        Element::Flex => true,
        Element::Grid => {
            let has = |property: &str| matches!(scene.current(node, property), Ok(Value::List(items)) if !items.is_empty());
            has("template_columns") || has("template_rows")
        }
        _ => false,
    })
}

/// The container half of a node's style: what it does to its children.
pub(crate) fn container_style(
    scene: &Scene,
    node: NodeHandle,
    style: &mut Style<String>,
) -> Result<(), LayoutError> {
    match scene.element(node)? {
        Element::Flex => {
            style.display = Display::Flex;
            style.flex_direction = match scene.string_value(node, "direction")? {
                "row" => FlexDirection::Row,
                "column" => FlexDirection::Column,
                "row_reverse" => FlexDirection::RowReverse,
                "column_reverse" => FlexDirection::ColumnReverse,
                other => return Err(bad("flex direction", other)),
            };
            style.flex_wrap = if scene.bool_value(node, "wrap")? {
                FlexWrap::Wrap
            } else {
                FlexWrap::NoWrap
            };
            let gap = gap_of(scene, node)?.max(0.0) as f32;
            style.gap = Size {
                width: LengthPercentage::length(gap),
                height: LengthPercentage::length(gap),
            };
            let padding = scene.number(node, "padding")?.max(0.0) as f32;
            style.padding = Rect {
                left: LengthPercentage::length(padding),
                right: LengthPercentage::length(padding),
                top: LengthPercentage::length(padding),
                bottom: LengthPercentage::length(padding),
            };
            style.align_items = align_items(&align_of(scene, node)?)?;
            style.justify_content = align_content(scene.string_value(node, "justify")?)?;
            style.align_content = align_content(scene.string_value(node, "align_content")?)?;
        }
        Element::Grid => {
            style.display = Display::Grid;
            style.grid_template_columns = tracks(scene.current(node, "template_columns")?)?;
            style.grid_template_rows = tracks(scene.current(node, "template_rows")?)?;
            style.gap = Size {
                width: LengthPercentage::length(
                    scene.number(node, "column_spacing")?.max(0.0) as f32
                ),
                height: LengthPercentage::length(scene.number(node, "row_spacing")?.max(0.0) as f32),
            };
            style.align_items = align_items(scene.string_value(node, "align")?)?;
            style.justify_content = align_content(scene.string_value(node, "justify")?)?;
        }
        _ => {}
    }
    Ok(())
}

/// The item half of a node's style: how it behaves inside its container.
///
/// Sizes come from the node's own `width` and `height` when set, else from
/// `layout.width` / `layout.height` (which may be a percent or `auto`),
/// else auto -- a leaf is then measured. The rest is the attached map.
pub(crate) fn item_style(scene: &Scene, node: NodeHandle) -> Result<Style<String>, LayoutError> {
    let attached = attached_layout(scene.current(node, "layout")?)?;
    let horizontal = match scene.parent(node)? {
        Some(parent) => match scene.element(parent)? {
            Element::Flex => scene.string_value(parent, "direction")?.starts_with("row"),
            _ => true,
        },
        None => true,
    };
    let read = Attached::read(attached, horizontal)?;
    let mut style = Style::<String>::default();
    let as_dimension = |bound: Option<Bound>| match bound {
        Some(Bound::Length(value)) => Dimension::length(value as f32),
        Some(Bound::Percent(fraction)) => Dimension::percent(fraction as f32),
        None => Dimension::auto(),
    };
    let as_bound = |bound: Option<Bound>| match bound {
        Some(Bound::Length(value)) => LengthPercentageAuto::length(value as f32),
        Some(Bound::Percent(fraction)) => LengthPercentageAuto::percent(fraction as f32),
        None => LengthPercentageAuto::auto(),
    };
    let own = |property: &str, preferred: Option<Bound>| -> Result<Dimension, LayoutError> {
        if let Some(pixels) = positive(scene.number(node, property)?) {
            return Ok(Dimension::length(pixels as f32));
        }
        if let Some(word) = attached.get(property)
            && matches!(word, Value::String(_))
        {
            return dimension(word, property);
        }
        Ok(as_dimension(preferred))
    };
    style.size = Size {
        width: own("width", read.preferred_width)?,
        height: own("height", read.preferred_height)?,
    };
    style.min_size = Size {
        width: as_bound(read.minimum_width),
        height: as_bound(read.minimum_height),
    };
    style.max_size = Size {
        width: as_bound(read.maximum_width),
        height: as_bound(read.maximum_height),
    };
    style.flex_grow = read.grow as f32;
    style.flex_shrink = read.shrink as f32;
    if let Some(basis) = attached.get("basis") {
        style.flex_basis = dimension(basis, "basis")?;
    }
    if let Some(word) = &read.align_self {
        style.align_self = align_items(word)?;
    }
    if let Some(margin) = attached.get("margin") {
        let margin = length_percentage_auto(margin, "margin")?;
        style.margin = Rect {
            left: margin,
            right: margin,
            top: margin,
            bottom: margin,
        };
    }
    style.grid_column = placement(attached, "column")?;
    style.grid_row = placement(attached, "row")?;
    container_style(scene, node, &mut style)?;
    Ok(style)
}
