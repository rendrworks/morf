//! A gradient as a configuration writes it and as the renderer reads it.
//!
//! The declarative form is one `gradient` property holding a map: a kind, an
//! angle, a centre, a colour space and a list of stops. It is one property
//! rather than a dozen scalars so that a configuration can hand the whole
//! thing to a binding, and so that a behavior on `gradient` moves every stop
//! at once — the canonical value is a tree of numbers and colours, which is
//! what the interpolator walks.

use std::collections::BTreeMap;

use crate::color::ColorSpace;
use crate::types::{Color, Value};

/// The most stops one gradient carries; the material has room for exactly
/// this many.
pub const MAX_GRADIENT_STOPS: usize = 16;

/// How a gradient runs across its rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GradientKind {
    /// Along a line at `angle`: 0 runs bottom to top, 90 left to right.
    #[default]
    Linear,
    /// Outwards from `at`, reaching `radius` of the rectangle.
    Radial,
    /// Around `at`, starting at `angle` and turning clockwise.
    Conic,
}

impl GradientKind {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "linear" => Some(Self::Linear),
            "radial" => Some(Self::Radial),
            "conic" => Some(Self::Conic),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Radial => "radial",
            Self::Conic => "conic",
        }
    }
}

/// One colour at one fraction of the gradient's run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    /// Where along the run, from 0 to 1.
    pub position: f64,
}

/// A gradient with its stops placed and its defaults filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    /// Degrees; the direction of a linear gradient or where a conic one starts.
    pub angle: f64,
    /// Centre of a radial or conic gradient as fractions of the rectangle.
    pub at: [f64; 2],
    /// Reach of a radial gradient as a fraction of the rectangle; `None`
    /// reaches the farthest corner.
    pub radius: Option<f64>,
    /// In order of position, at least two and at most sixteen.
    pub stops: Vec<GradientStop>,
    /// The space neighbouring stops are mixed in.
    pub space: ColorSpace,
}

impl Gradient {
    /// Reads a gradient from its declarative value. Nil and an empty table
    /// mean there is none.
    pub fn parse(value: &Value) -> Result<Option<Self>, String> {
        let entries = match value {
            Value::Nil => return Ok(None),
            Value::List(items) if items.is_empty() => return Ok(None),
            Value::Map(entries) if entries.is_empty() => return Ok(None),
            Value::Map(entries) => entries,
            _ => return Err("a gradient is a table".to_owned()),
        };
        for key in entries.keys() {
            if !matches!(
                key.as_str(),
                "kind" | "angle" | "at" | "radius" | "stops" | "space"
            ) {
                return Err(format!("a gradient has no `{key}`"));
            }
        }
        let kind = match entries.get("kind") {
            None => GradientKind::Linear,
            Some(Value::String(name)) => GradientKind::parse(name)
                .ok_or_else(|| format!("gradient kind `{name}` is not linear, radial or conic"))?,
            Some(_) => return Err("gradient kind must be linear, radial or conic".to_owned()),
        };
        let angle = match entries.get("angle") {
            None => match kind {
                GradientKind::Linear => 180.0,
                _ => 0.0,
            },
            Some(Value::Number(angle)) if angle.is_finite() => *angle,
            Some(_) => return Err("gradient angle must be a number".to_owned()),
        };
        let at = match entries.get("at") {
            None => [0.5, 0.5],
            Some(value) => point(value).ok_or("gradient `at` is { x, y }")?,
        };
        let radius = match entries.get("radius") {
            None => None,
            Some(Value::Number(radius)) if radius.is_finite() && *radius > 0.0 => Some(*radius),
            Some(_) => return Err("gradient radius must be a positive number".to_owned()),
        };
        let space = match entries.get("space") {
            None => ColorSpace::Oklab,
            Some(Value::String(name)) => ColorSpace::parse(name)
                .ok_or_else(|| format!("gradient space `{name}` is not srgb, oklab or oklch"))?,
            Some(_) => return Err("gradient space must be srgb, oklab or oklch".to_owned()),
        };
        let stops = match entries.get("stops") {
            Some(Value::List(items)) => stops(items)?,
            _ => return Err("a gradient needs a list of stops".to_owned()),
        };
        Ok(Some(Self {
            kind,
            angle,
            at,
            radius,
            stops,
            space,
        }))
    }

    /// The declarative value this gradient reads back as: every default
    /// written out, every stop a `{ color, position }` map, so that two of
    /// them interpolate field by field.
    pub fn to_value(&self) -> Value {
        let mut entries = BTreeMap::new();
        entries.insert(
            "kind".to_owned(),
            Value::String(self.kind.name().to_owned()),
        );
        entries.insert("angle".to_owned(), Value::Number(self.angle));
        entries.insert(
            "at".to_owned(),
            Value::List(vec![Value::Number(self.at[0]), Value::Number(self.at[1])]),
        );
        if let Some(radius) = self.radius {
            entries.insert("radius".to_owned(), Value::Number(radius));
        }
        entries.insert(
            "space".to_owned(),
            Value::String(self.space.name().to_owned()),
        );
        let stops = self
            .stops
            .iter()
            .map(|stop| {
                let mut entry = BTreeMap::new();
                entry.insert("color".to_owned(), Value::Color(stop.color));
                entry.insert("position".to_owned(), Value::Number(stop.position));
                Value::Map(entry)
            })
            .collect();
        entries.insert("stops".to_owned(), Value::List(stops));
        Value::Map(entries)
    }

    /// The value a `gradient` property stores: what `parse` would give back
    /// unchanged, or an empty map for no gradient.
    pub(crate) fn canonical(value: Value) -> Result<Value, String> {
        Ok(match Self::parse(&value)? {
            None => Value::Map(BTreeMap::new()),
            Some(gradient) => gradient.to_value(),
        })
    }
}

fn point(value: &Value) -> Option<[f64; 2]> {
    let (x, y) = match value {
        Value::List(items) => match items.as_slice() {
            [x, y] => (x, y),
            _ => return None,
        },
        Value::Map(entries) => (entries.get("x")?, entries.get("y")?),
        _ => return None,
    };
    match (x, y) {
        (Value::Number(x), Value::Number(y)) if x.is_finite() && y.is_finite() => Some([*x, *y]),
        _ => None,
    }
}

fn stop_color(value: &Value) -> Result<Color, String> {
    match value {
        Value::Color(color) => Ok(*color),
        Value::String(text) => {
            Color::parse(text).ok_or_else(|| format!("`{text}` is not a colour"))
        }
        _ => Err("a gradient stop needs a colour".to_owned()),
    }
}

fn stop_position(value: Option<&Value>) -> Result<Option<f64>, String> {
    match value {
        None | Some(Value::Nil) => Ok(None),
        Some(Value::Number(position)) if position.is_finite() => Ok(Some(*position)),
        Some(_) => Err("a gradient stop position is a number".to_owned()),
    }
}

/// Reads the stop list. A stop is a colour, a `{ color, position }` pair or a
/// `{ color = , position = }` map; a missing position is spread evenly
/// between its placed neighbours, the way a stylesheet places them.
fn stops(items: &[Value]) -> Result<Vec<GradientStop>, String> {
    if items.len() < 2 {
        return Err("a gradient needs at least two stops".to_owned());
    }
    if items.len() > MAX_GRADIENT_STOPS {
        return Err(format!(
            "a gradient takes at most {MAX_GRADIENT_STOPS} stops"
        ));
    }
    let mut placed = Vec::with_capacity(items.len());
    for item in items {
        placed.push(match item {
            Value::List(pair) => match pair.as_slice() {
                [color] => (stop_color(color)?, None),
                [color, position] => (stop_color(color)?, stop_position(Some(position))?),
                _ => return Err("a gradient stop is { color, position }".to_owned()),
            },
            Value::Map(entry) => {
                let color = entry.get("color").ok_or("a gradient stop needs a colour")?;
                (stop_color(color)?, stop_position(entry.get("position"))?)
            }
            other => (stop_color(other)?, None),
        });
    }
    let last = placed.len() - 1;
    placed[0].1.get_or_insert(0.0);
    placed[last].1.get_or_insert(1.0);
    let mut stops = Vec::with_capacity(placed.len());
    let mut floor: f64 = 0.0;
    let mut index = 0;
    while index < placed.len() {
        let (color, position) = placed[index];
        match position {
            Some(position) => {
                // A stop before an earlier one is pulled up to it, so the run
                // never goes backwards and a repeated position is a hard edge.
                floor = position.clamp(0.0, 1.0).max(floor);
                stops.push(GradientStop {
                    color,
                    position: floor,
                });
                index += 1;
            }
            None => {
                let next = (index + 1..placed.len())
                    .find(|&later| placed[later].1.is_some())
                    .expect("the last stop is placed");
                let to = placed[next]
                    .1
                    .expect("found placed")
                    .clamp(0.0, 1.0)
                    .max(floor);
                let gaps = (next - index + 1) as f64;
                for (step, (color, _)) in placed[index..next].iter().enumerate() {
                    let position = floor + (to - floor) * (step + 1) as f64 / gaps;
                    stops.push(GradientStop {
                        color: *color,
                        position,
                    });
                }
                index = next;
            }
        }
    }
    Ok(stops)
}
