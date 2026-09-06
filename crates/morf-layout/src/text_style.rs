//! How a run of text is set: its line height, spacing, slant and width.
//!
//! Read once from a node and carried into measurement and painting alike, so
//! the two cannot disagree about what a line of it takes.

use morf_scene::{NodeHandle, Scene, Value};

use crate::helpers::LayoutError;

/// The distance from one baseline to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    /// So many times the font size, the way a stylesheet's bare number is.
    Multiple(f64),
    /// A size in logical pixels, written `"24px"`.
    Pixels(f64),
}

impl LineHeight {
    /// The line height for a font size.
    pub fn pixels(self, size: f64) -> f64 {
        match self {
            Self::Multiple(multiple) => size * multiple,
            Self::Pixels(pixels) => pixels,
        }
    }

    /// Reads a line height from its declarative value.
    pub fn parse(value: &Value) -> Result<Self, String> {
        match value {
            Value::Number(multiple) if multiple.is_finite() && *multiple > 0.0 => {
                Ok(Self::Multiple(*multiple))
            }
            Value::String(text) => text
                .strip_suffix("px")
                .and_then(|pixels| pixels.trim().parse::<f64>().ok())
                .filter(|pixels| pixels.is_finite() && *pixels > 0.0)
                .map(Self::Pixels)
                .ok_or_else(|| format!("`{text}` is not a line height")),
            _ => Err("line_height is a multiple of the font size or a `px` size".to_owned()),
        }
    }
}

impl Default for LineHeight {
    fn default() -> Self {
        Self::Multiple(1.2)
    }
}

/// The slant of a face.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

impl FontStyle {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "normal" => Some(Self::Normal),
            "italic" => Some(Self::Italic),
            "oblique" => Some(Self::Oblique),
            _ => None,
        }
    }
}

/// The width of a face, from the narrowest cut to the widest.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FontStretch {
    UltraCondensed,
    ExtraCondensed,
    Condensed,
    SemiCondensed,
    #[default]
    Normal,
    SemiExpanded,
    Expanded,
    ExtraExpanded,
    UltraExpanded,
}

impl FontStretch {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "ultra_condensed" => Some(Self::UltraCondensed),
            "extra_condensed" => Some(Self::ExtraCondensed),
            "condensed" => Some(Self::Condensed),
            "semi_condensed" => Some(Self::SemiCondensed),
            "normal" => Some(Self::Normal),
            "semi_expanded" => Some(Self::SemiExpanded),
            "expanded" => Some(Self::Expanded),
            "extra_expanded" => Some(Self::ExtraExpanded),
            "ultra_expanded" => Some(Self::UltraExpanded),
            _ => None,
        }
    }
}

/// Everything about how text is set besides its family, size and weight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextStyle {
    pub line_height: LineHeight,
    /// Added between letters, in logical pixels.
    pub letter_spacing: f64,
    /// Added at every space, in logical pixels.
    pub word_spacing: f64,
    pub font_style: FontStyle,
    pub font_stretch: FontStretch,
}

impl TextStyle {
    /// Reads a text node's style.
    pub fn from_scene(scene: &Scene, node: NodeHandle) -> Result<Self, LayoutError> {
        let font_style = scene.string_value(node, "font_style")?;
        let font_stretch = scene.string_value(node, "font_stretch")?;
        Ok(Self {
            line_height: LineHeight::parse(scene.current(node, "line_height")?)
                .map_err(|message| LayoutError::Scene(format!("Text: {message}")))?,
            letter_spacing: scene.number(node, "letter_spacing")?,
            word_spacing: scene.number(node, "word_spacing")?,
            font_style: FontStyle::parse(font_style).ok_or_else(|| {
                LayoutError::Scene(format!(
                    "Text: font_style `{font_style}` is not normal, italic or oblique"
                ))
            })?,
            font_stretch: FontStretch::parse(font_stretch).ok_or_else(|| {
                LayoutError::Scene(format!(
                    "Text: font_stretch `{font_stretch}` is not a width from ultra_condensed to ultra_expanded"
                ))
            })?,
        })
    }

    /// The style as a key: the numbers by their bits, so two equal styles
    /// hash alike.
    pub fn key(&self) -> TextStyleKey {
        let (line_kind, line_value) = match self.line_height {
            LineHeight::Multiple(value) => (0, value.to_bits()),
            LineHeight::Pixels(value) => (1, value.to_bits()),
        };
        TextStyleKey {
            line_kind,
            line_value,
            letter_spacing: self.letter_spacing.to_bits(),
            word_spacing: self.word_spacing.to_bits(),
            font_style: self.font_style,
            font_stretch: self.font_stretch,
        }
    }
}

/// A text style as a hashable key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextStyleKey {
    line_kind: u8,
    line_value: u64,
    letter_spacing: u64,
    word_spacing: u64,
    font_style: FontStyle,
    font_stretch: FontStretch,
}
