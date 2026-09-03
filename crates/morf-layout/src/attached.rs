//! The attached `layout` map, read one way by every container.
//!
//! A child says how it wants to be treated in the map its parent reads:
//! `grow` for a share of the leftover space (`fill_width`, `fill_height`
//! and `stretch` are the older words and mean the same), `shrink`,
//! `basis`, `align_self` (`alignment` is the older word), `margin`, a
//! `width` or `height` the container may give it, `minimum_*` and
//! `maximum_*` as numbers or percent strings, and for a grid `column`,
//! `row` and the spans. One parser, so a word means the same thing under
//! every parent.

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
    pub(crate) fill_cross: bool,
    pub(crate) align_self: Option<String>,
    pub(crate) preferred_width: Option<Bound>,
    pub(crate) preferred_height: Option<Bound>,
    pub(crate) minimum_width: Option<Bound>,
    pub(crate) minimum_height: Option<Bound>,
    pub(crate) maximum_width: Option<Bound>,
    pub(crate) maximum_height: Option<Bound>,
}

impl Attached {
    /// Reads a node's map. `horizontal` says which of `fill_width` and
    /// `fill_height` is the main axis, for the older fill words.
    pub(crate) fn read(
        map: &BTreeMap<String, Value>,
        horizontal: bool,
    ) -> Result<Self, LayoutError> {
        let number = |key: &str| match map.get(key) {
            Some(Value::Number(value)) if value.is_finite() => Some(value.max(0.0)),
            _ => None,
        };
        let flag = |key: &str| matches!(map.get(key), Some(Value::Bool(true)));
        let (fill_main, fill_cross) = if horizontal {
            ("fill_width", "fill_height")
        } else {
            ("fill_height", "fill_width")
        };
        let mut attached = Self {
            grow: number("grow")
                .or_else(|| number("stretch"))
                .unwrap_or(if flag(fill_main) { 1.0 } else { 0.0 }),
            shrink: number("shrink").unwrap_or(1.0),
            fill_cross: flag(fill_cross),
            align_self: None,
            ..Self::default()
        };
        if let Some(Value::String(word)) = map.get("align_self").or_else(|| map.get("alignment")) {
            attached.align_self = Some(word.clone());
        }
        if attached.fill_cross && attached.align_self.is_none() {
            attached.align_self = Some("stretch".to_owned());
        }
        let read = |primary: &str, alias: &str| -> Result<Option<Bound>, LayoutError> {
            match map.get(primary).or_else(|| map.get(alias)) {
                Some(value) => bound(value, primary),
                None => Ok(None),
            }
        };
        attached.preferred_width = read("width", "preferred_width")?;
        attached.preferred_height = read("height", "preferred_height")?;
        attached.minimum_width = read("minimum_width", "min_width")?;
        attached.minimum_height = read("minimum_height", "min_height")?;
        attached.maximum_width = read("maximum_width", "max_width")?;
        attached.maximum_height = read("maximum_height", "max_height")?;
        Ok(attached)
    }
}
