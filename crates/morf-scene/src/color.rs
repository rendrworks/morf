//! Colour: what a string means, and how two colours are between.
//!
//! `pastel` reads every syntax a shell author writes -- hex, `rgb()`,
//! `hsl()`, `lab()`, `lch()`, `oklab()`, `oklch()`, `gray()`, the 148 CSS
//! names -- and this adds the handful CSS has that it does not: the
//! slash-alpha forms, `hwb()`, `transparent`, and `0x` hex. The scene keeps
//! a colour as four sRGB floats because the renderer wants that; the value a
//! configuration holds in Lua is `pastel`'s, and the two convert without
//! loss beyond eight bits.
//!
//! Interpolation happens in OkLab on premultiplied components unless a
//! behavior names another space: a fade through transparent does not pass
//! through grey, and the midpoint of two saturated hues is not the muddy one
//! gamma-space lerping gives.

use crate::types::Color;

/// The space two colours are interpolated in.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorSpace {
    /// Gamma-encoded sRGB, per channel: what CSS did for twenty years.
    Srgb,
    /// Perceptual, rectangular: the default.
    #[default]
    Oklab,
    /// Perceptual, polar: hue travels as an angle.
    Oklch,
}

/// Which way round the hue wheel a polar interpolation goes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HueDirection {
    #[default]
    Shorter,
    Longer,
}

impl ColorSpace {
    /// Parses the name a configuration uses.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "srgb" => Some(Self::Srgb),
            "oklab" => Some(Self::Oklab),
            "oklch" => Some(Self::Oklch),
            _ => None,
        }
    }
}

impl HueDirection {
    /// Parses the name a configuration uses.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "shorter" => Some(Self::Shorter),
            "longer" => Some(Self::Longer),
            _ => None,
        }
    }
}

impl Color {
    /// The colour as `pastel` holds it.
    pub fn to_pastel(self) -> pastel::Color {
        pastel::Color::from_rgba_float(
            f64::from(self.red),
            f64::from(self.green),
            f64::from(self.blue),
            f64::from(self.alpha),
        )
    }

    /// A `pastel` colour as the scene holds it.
    pub fn from_pastel(color: &pastel::Color) -> Self {
        let rgba = color.to_rgba_float();
        Self {
            red: rgba.r as f32,
            green: rgba.g as f32,
            blue: rgba.b as f32,
            alpha: rgba.alpha as f32,
        }
    }
}

/// Reads a colour from the forms a configuration writes.
pub(crate) fn parse(input: &str) -> Option<Color> {
    let input = input.trim();
    if input.eq_ignore_ascii_case("transparent") {
        return Some(Color::rgba8(0, 0, 0, 0));
    }
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        return pastel::parser::parse_color(&format!("#{hex}")).map(|c| Color::from_pastel(&c));
    }
    if let Some(color) = hwb(input) {
        return Some(color);
    }
    let normalised = slash_alpha(input).unwrap_or_else(|| input.to_owned());
    pastel::parser::parse_color(&normalised).map(|c| Color::from_pastel(&c))
}

/// `rgb(255 136 0 / 50%)` and `hsl(30 100% 50% / .5)` into the comma forms
/// `pastel` reads.
fn slash_alpha(input: &str) -> Option<String> {
    let open = input.find('(')?;
    let close = input.strip_suffix(')')?;
    let name = input[..open].trim().to_ascii_lowercase();
    let inner = &close[open + 1..];
    let (channels, alpha) = match inner.split_once('/') {
        Some((channels, alpha)) => (channels, Some(alpha.trim())),
        None => (inner, None),
    };
    let base = match name.as_str() {
        "rgb" | "rgba" => "rgba",
        "hsl" | "hsla" => "hsla",
        "hsv" | "hsva" => "hsva",
        _ => return None,
    };
    let mut parts = channels
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    if let Some(alpha) = alpha {
        parts.push(alpha.to_owned());
    }
    Some(format!("{base}({})", parts.join(", ")))
}

/// `hwb(h w% b%)`: hue, whiteness, blackness, which CSS has and `pastel`
/// does not.
fn hwb(input: &str) -> Option<Color> {
    let inner = input
        .strip_prefix("hwb(")
        .or_else(|| input.strip_prefix("HWB("))?
        .strip_suffix(')')?;
    let (channels, alpha) = match inner.split_once('/') {
        Some((channels, alpha)) => (channels, alpha.trim()),
        None => (inner, "1"),
    };
    let parts = channels
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    let hue = parts[0]
        .trim_end_matches("deg")
        .parse::<f64>()
        .ok()?
        .rem_euclid(360.0);
    let percent = |part: &str| {
        part.strip_suffix('%')?
            .parse::<f64>()
            .ok()
            .map(|v| v / 100.0)
    };
    let (mut white, mut black) = (percent(parts[1])?, percent(parts[2])?);
    let alpha = if let Some(percent) = alpha.strip_suffix('%') {
        percent.parse::<f64>().ok()? / 100.0
    } else {
        alpha.parse::<f64>().ok()?
    };
    if white + black > 1.0 {
        let sum = white + black;
        white /= sum;
        black /= sum;
    }
    let pure = pastel::Color::from_hsl(hue, 1.0, 0.5).to_rgba_float();
    let channel = |value: f64| (value * (1.0 - white - black) + white).clamp(0.0, 1.0) as f32;
    Some(Color {
        red: channel(pure.r),
        green: channel(pure.g),
        blue: channel(pure.b),
        alpha: alpha.clamp(0.0, 1.0) as f32,
    })
}

fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB to OkLab, Björn Ottosson's matrices.
pub(crate) fn to_oklab(color: Color) -> [f64; 3] {
    let r = srgb_to_linear(f64::from(color.red));
    let g = srgb_to_linear(f64::from(color.green));
    let b = srgb_to_linear(f64::from(color.blue));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// OkLab back to sRGB, clamped into gamut.
pub(crate) fn from_oklab(lab: [f64; 3], alpha: f64) -> Color {
    let [big_l, a, b] = lab;
    let l = (big_l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (big_l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (big_l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    let r = 4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s;
    let g = -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s;
    let b = -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s;
    Color {
        red: linear_to_srgb(r) as f32,
        green: linear_to_srgb(g) as f32,
        blue: linear_to_srgb(b) as f32,
        alpha: alpha.clamp(0.0, 1.0) as f32,
    }
}

/// A colour as four numbers in `space`, premultiplied by alpha, which is
/// what interpolation and physics move.
///
/// For OkLCh the third number is the hue in degrees and is not
/// premultiplied; a hue has no magnitude to scale.
pub(crate) fn coords(color: Color, space: ColorSpace) -> [f64; 4] {
    let alpha = f64::from(color.alpha);
    match space {
        ColorSpace::Srgb => [
            f64::from(color.red) * alpha,
            f64::from(color.green) * alpha,
            f64::from(color.blue) * alpha,
            alpha,
        ],
        ColorSpace::Oklab => {
            let [l, a, b] = to_oklab(color);
            [l * alpha, a * alpha, b * alpha, alpha]
        }
        ColorSpace::Oklch => {
            let [l, a, b] = to_oklab(color);
            let chroma = (a * a + b * b).sqrt();
            let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
            [l * alpha, chroma * alpha, hue, alpha]
        }
    }
}

/// The colour those four numbers name.
pub(crate) fn from_coords(coords: [f64; 4], space: ColorSpace) -> Color {
    let alpha = coords[3].clamp(0.0, 1.0);
    let un = |value: f64| if alpha > 0.0 { value / alpha } else { 0.0 };
    match space {
        ColorSpace::Srgb => Color {
            red: un(coords[0]).clamp(0.0, 1.0) as f32,
            green: un(coords[1]).clamp(0.0, 1.0) as f32,
            blue: un(coords[2]).clamp(0.0, 1.0) as f32,
            alpha: alpha as f32,
        },
        ColorSpace::Oklab => from_oklab([un(coords[0]), un(coords[1]), un(coords[2])], alpha),
        ColorSpace::Oklch => {
            let (l, chroma) = (un(coords[0]), un(coords[1]));
            let hue = coords[2].to_radians();
            from_oklab([l, chroma * hue.cos(), chroma * hue.sin()], alpha)
        }
    }
}

/// Puts the second hue on the side of the first that the direction asks
/// for, so a linear step between them travels that way.
pub(crate) fn align_hue(from: f64, to: f64, direction: HueDirection) -> f64 {
    let delta = (to - from).rem_euclid(360.0);
    match direction {
        HueDirection::Shorter => {
            if delta > 180.0 {
                from + delta - 360.0
            } else {
                from + delta
            }
        }
        HueDirection::Longer => {
            if delta < 180.0 {
                from + delta - 360.0
            } else {
                from + delta
            }
        }
    }
}

/// The colour a fraction of the way from one to another.
pub fn mix(from: Color, to: Color, progress: f64, space: ColorSpace, hue: HueDirection) -> Color {
    let a = coords(from, space);
    let mut b = coords(to, space);
    if space == ColorSpace::Oklch {
        b[2] = align_hue(a[2], b[2], hue);
    }
    let mixed = std::array::from_fn(|index| a[index] + (b[index] - a[index]) * progress);
    from_coords(mixed, space)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Color, b: Color) -> bool {
        (a.red - b.red).abs() < 0.01
            && (a.green - b.green).abs() < 0.01
            && (a.blue - b.blue).abs() < 0.01
            && (a.alpha - b.alpha).abs() < 0.01
    }

    #[test]
    fn every_syntax_reads() {
        let orange = Color::rgba8(255, 136, 0, 255);
        for text in [
            "#ff8800",
            "#f80",
            "ff8800",
            "0xff8800",
            "rgb(255, 136, 0)",
            "rgb(255 136 0)",
            "rgb(255 136 0 / 100%)",
            "hsl(32, 100%, 50%)",
            "hsl(32 100% 50% / 1)",
            "hwb(32 0% 0%)",
        ] {
            let parsed = parse(text).unwrap_or_else(|| panic!("{text}"));
            assert!(close(parsed, orange), "{text}: {parsed:?}");
        }
        assert_eq!(parse("transparent"), Some(Color::rgba8(0, 0, 0, 0)));
        assert!(close(
            parse("rebeccapurple").unwrap(),
            Color::rgba8(102, 51, 153, 255)
        ));
        assert!((parse("rgba(255 136 0 / 50%)").unwrap().alpha - 0.5).abs() < 0.01);
        assert!(parse("oklch(0.7 0.18 56)").is_some());
        assert!(parse("lab(69, 39, 75)").is_some());
        assert_eq!(parse("#éé"), None);
        assert_eq!(parse("nope"), None);
    }

    #[test]
    fn oklab_round_trips_and_mixes_perceptually() {
        let c = Color::rgba8(37, 190, 120, 255);
        assert!(close(from_oklab(to_oklab(c), 1.0), c));
        // Halfway from opaque red to transparent is half-covered red, not
        // half-covered grey: premultiplied.
        let mid = mix(
            Color::rgba8(255, 0, 0, 255),
            Color::rgba8(0, 0, 0, 0),
            0.5,
            ColorSpace::Oklab,
            HueDirection::Shorter,
        );
        assert!((mid.alpha - 0.5).abs() < 0.01);
        assert!(mid.red > 0.95, "{mid:?}");
        // Around the wheel the long way passes through the far side.
        let long = mix(
            Color::rgba8(255, 0, 0, 255),
            Color::rgba8(0, 0, 255, 255),
            0.5,
            ColorSpace::Oklch,
            HueDirection::Longer,
        );
        let short = mix(
            Color::rgba8(255, 0, 0, 255),
            Color::rgba8(0, 0, 255, 255),
            0.5,
            ColorSpace::Oklch,
            HueDirection::Shorter,
        );
        assert!(!close(long, short));
    }
}
