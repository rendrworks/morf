//! A line drawn with text: under it, over it or through it.

use std::collections::BTreeMap;

use crate::types::{Color, Value};

/// Where the line sits against the glyphs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecorationLine {
    #[default]
    Under,
    Over,
    Through,
}

impl DecorationLine {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "under" => Some(Self::Under),
            "over" => Some(Self::Over),
            "through" => Some(Self::Through),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Under => "under",
            Self::Over => "over",
            Self::Through => "through",
        }
    }
}

/// A text decoration with its defaults left to the font where it has them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextDecoration {
    pub line: DecorationLine,
    /// Logical pixels; `None` takes the face's own stroke.
    pub thickness: Option<f64>,
    /// Logical pixels added to where the face puts the line, downwards.
    pub offset: f64,
    /// `None` draws in the text's colour.
    pub color: Option<Color>,
}

impl TextDecoration {
    /// Reads a decoration from its declarative value. Nil and an empty table
    /// mean there is none.
    pub fn parse(value: &Value) -> Result<Option<Self>, String> {
        let entries = match value {
            Value::Nil => return Ok(None),
            Value::List(items) if items.is_empty() => return Ok(None),
            Value::Map(entries) if entries.is_empty() => return Ok(None),
            Value::Map(entries) => entries,
            _ => return Err("a decoration is a table".to_owned()),
        };
        for key in entries.keys() {
            if !matches!(key.as_str(), "line" | "thickness" | "offset" | "color") {
                return Err(format!("a decoration has no `{key}`"));
            }
        }
        let line = match entries.get("line") {
            None => DecorationLine::Under,
            Some(Value::String(name)) => DecorationLine::parse(name)
                .ok_or_else(|| format!("decoration line `{name}` is not under, over or through"))?,
            Some(_) => return Err("decoration line must be under, over or through".to_owned()),
        };
        let thickness = match entries.get("thickness") {
            None => None,
            Some(Value::Number(value)) if value.is_finite() && *value > 0.0 => Some(*value),
            Some(_) => return Err("decoration thickness must be a positive number".to_owned()),
        };
        let offset = match entries.get("offset") {
            None => 0.0,
            Some(Value::Number(value)) if value.is_finite() => *value,
            Some(_) => return Err("decoration offset must be a number".to_owned()),
        };
        let color = match entries.get("color") {
            None => None,
            Some(Value::Color(color)) => Some(*color),
            Some(Value::String(text)) => {
                Some(Color::parse(text).ok_or_else(|| format!("`{text}` is not a colour"))?)
            }
            Some(_) => return Err("decoration color must be a colour".to_owned()),
        };
        Ok(Some(Self {
            line,
            thickness,
            offset,
            color,
        }))
    }

    /// The declarative value this reads back as, every default written out.
    pub fn to_value(&self) -> Value {
        let mut entries = BTreeMap::new();
        entries.insert(
            "line".to_owned(),
            Value::String(self.line.name().to_owned()),
        );
        if let Some(thickness) = self.thickness {
            entries.insert("thickness".to_owned(), Value::Number(thickness));
        }
        entries.insert("offset".to_owned(), Value::Number(self.offset));
        if let Some(color) = self.color {
            entries.insert("color".to_owned(), Value::Color(color));
        }
        Value::Map(entries)
    }

    pub(crate) fn canonical(value: Value) -> Result<Value, String> {
        Ok(match Self::parse(&value)? {
            None => Value::Map(BTreeMap::new()),
            Some(decoration) => decoration.to_value(),
        })
    }
}
