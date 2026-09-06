//! The attached `layout` map, read one way by every container.
//!
//! A child says how it wants to be treated in the map its parent reads:
//! `grow` for a share of the leftover space, `shrink`, `basis`,
//! `align_self`, `margin`, a `width` or `height` the container may give
//! it, `minimum_*` and `maximum_*` as numbers or percent strings, and for
//! a grid `column`, `row` and the spans. One parser, so a word means the
//! same thing under every parent.

use std::collections::BTreeMap;

use morf_scene::Value;

use crate::helpers::LayoutError;

/// A size bound: pixels, or a fraction of the parent's extent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Bound {
    Length(f64),
    Percent(f64),
}

fn bound(value: &Value, key: &str) -> Result<Option<Bound>, LayoutError> {
    match value {
        Value::Nil => Ok(None),
        Value::Number(pixels) if pixels.is_finite() && *pixels >= 0.0 => {
            Ok(Some(Bound::Length(*pixels)))
        }
        Value::String(word) => word
            .strip_suffix('%')
            .and_then(|number| number.trim().parse::<f64>().ok())
            .map(|percent| Some(Bound::Percent(percent / 100.0)))
            .ok_or_else(|| LayoutError::Scene(format!("layout.{key}: unknown size `{word}`"))),
        other => Err(LayoutError::Scene(format!(
            "layout.{key} must be a number or a percent, not {other:?}"
        ))),
    }
}

/// What a child asked of its container.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Attached {
    pub(crate) grow: f64,
    pub(crate) shrink: f64,
    pub(crate) align_self: Option<String>,
    pub(crate) preferred_width: Option<Bound>,
    pub(crate) preferred_height: Option<Bound>,
    pub(crate) minimum_width: Option<Bound>,
    pub(crate) minimum_height: Option<Bound>,
    pub(crate) maximum_width: Option<Bound>,
    pub(crate) maximum_height: Option<Bound>,
}

impl Attached {
    /// Reads a node's map. A key that no container reads is an error that
    /// names it, so a word from another vocabulary is never quietly ignored.
    pub(crate) fn read(map: &BTreeMap<String, Value>) -> Result<Self, LayoutError> {
        const KNOWN: [&str; 15] = [
            "grow",
            "shrink",
            "basis",
            "align_self",
            "margin",
            "width",
            "height",
            "minimum_width",
            "minimum_height",
            "maximum_width",
            "maximum_height",
            "column",
            "row",
            "column_span",
            "row_span",
        ];
        if let Some(unknown) = map.keys().find(|key| !KNOWN.contains(&key.as_str())) {
            return Err(LayoutError::Scene(format!(
                "unknown layout key `{unknown}`: the keys are {}",
                KNOWN.join(", ")
            )));
        }
        let number = |key: &str| match map.get(key) {
            Some(Value::Number(value)) if value.is_finite() => Some(value.max(0.0)),
            _ => None,
        };
        let mut attached = Self {
            grow: number("grow").unwrap_or(0.0),
            shrink: number("shrink").unwrap_or(1.0),
            align_self: None,
            ..Self::default()
        };
        if let Some(Value::String(word)) = map.get("align_self") {
            attached.align_self = Some(word.clone());
        }
        let read = |key: &str| -> Result<Option<Bound>, LayoutError> {
            match map.get(key) {
                Some(value) => bound(value, key),
                None => Ok(None),
            }
        };
        attached.preferred_width = read("width")?;
        attached.preferred_height = read("height")?;
        attached.minimum_width = read("minimum_width")?;
        attached.minimum_height = read("minimum_height")?;
        attached.maximum_width = read("maximum_width")?;
        attached.maximum_height = read("maximum_height")?;
        Ok(attached)
    }
}
