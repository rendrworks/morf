//! What a colour value can do: convert, change, measure.
//!
//! Every method returns a new value; a colour is never changed in place,
//! which is what lets one sit in a theme table and be lightened by three
//! bindings at once.

use luna::{Callback, CallbackReturn, Context, Table, UserRef, Value as LuaValue};
use pastel::{Color, Format, Fraction, HSLA, HSVA, LCh, Lab, OkLCh, OkLab, RGBA};

use crate::api_color::{ColorToken, color_of, color_userdata};
use crate::scene_bindings::*;

/// Mixes two colours in the named space; `oklab` when none is named.
pub(crate) fn mix_in(
    a: &Color,
    b: &Color,
    fraction: f64,
    space: Option<&str>,
) -> Result<Color, String> {
    let fraction = Fraction::from(fraction);
    Ok(match space.unwrap_or("oklab") {
        "rgb" | "srgb" => a.mix::<RGBA<f64>>(b, fraction),
        "hsl" => a.mix::<HSLA>(b, fraction),
        "hsv" => a.mix::<HSVA>(b, fraction),
        "lab" => a.mix::<Lab>(b, fraction),
        "oklab" => a.mix::<OkLab>(b, fraction),
        "lch" => a.mix::<LCh>(b, fraction),
        "oklch" => a.mix::<OkLCh>(b, fraction),
        other => return Err(format!("unknown mixing space `{other}`")),
    })
}

fn with_alpha(color: &Color, alpha: f64) -> Color {
    let hsla = color.to_hsla();
    Color::from_hsla(hsla.h, hsla.s, hsla.l, alpha.clamp(0.0, 1.0))
}

fn table_of<'gc>(ctx: Context<'gc>, entries: &[(&'static str, f64)]) -> Table<'gc> {
    let table = Table::new(&ctx);
    for (key, value) in entries {
        table.set_field(ctx, key, *value);
    }
    table
}

/// `c:with { l = 0.6 }`: the colour with some channels replaced, in the
/// space the keys name or `space` says.
fn with_channels<'gc>(
    ctx: Context<'gc>,
    color: &Color,
    changes: Table<'gc>,
) -> Result<Color, String> {
    let number = |key: &str| match changes.get_value(ctx, key) {
        LuaValue::Integer(value) => Some(value as f64),
        LuaValue::Number(value) if value.is_finite() => Some(value),
        _ => None,
    };
    let space = match changes.get_value(ctx, "space") {
        LuaValue::String(name) => name.display_lossy().to_string(),
        _ => {
            if number("r").is_some() || number("g").is_some() || number("b").is_some() {
                "rgb".to_owned()
            } else if number("v").is_some() {
                "hsv".to_owned()
            } else if number("s").is_some() {
                "hsl".to_owned()
            } else if number("c").is_some() || number("h").is_some() {
                "oklch".to_owned()
            } else {
                "oklab".to_owned()
            }
        }
    };
    let alpha = number("a").or_else(|| number("alpha"));
    Ok(match space.as_str() {
        "rgb" => {
            let rgba = color.to_rgba_float();
            Color::from_rgba_float(
                number("r").unwrap_or(rgba.r),
                number("g").unwrap_or(rgba.g),
                number("b").unwrap_or(rgba.b),
                alpha.unwrap_or(rgba.alpha),
            )
        }
        "hsl" => {
            let hsla = color.to_hsla();
            Color::from_hsla(
                number("h").unwrap_or(hsla.h),
                number("s").unwrap_or(hsla.s),
                number("l").unwrap_or(hsla.l),
                alpha.unwrap_or(hsla.alpha),
            )
        }
        "hsv" => {
            let hsva = color.to_hsva();
            Color::from_hsva(
                number("h").unwrap_or(hsva.h),
                number("s").unwrap_or(hsva.s),
                number("v").unwrap_or(hsva.v),
                alpha.unwrap_or(hsva.alpha),
            )
        }
        "lab" => {
            let lab = color.to_lab();
            Color::from_lab(
                number("l").unwrap_or(lab.l),
                number("a").unwrap_or(lab.a),
                number("b").unwrap_or(lab.b),
                alpha.unwrap_or(lab.alpha),
            )
        }
        "oklab" => {
            let lab = color.to_oklab();
            Color::from_oklab(
                number("l").unwrap_or(lab.l),
                number("a").unwrap_or(lab.a),
                number("b").unwrap_or(lab.b),
                alpha.unwrap_or(lab.alpha),
            )
        }
        "lch" => {
            let lch = color.to_lch();
            Color::from_lch(
                number("l").unwrap_or(lch.l),
                number("c").unwrap_or(lch.c),
                number("h").unwrap_or(lch.h),
                alpha.unwrap_or(lch.alpha),
            )
        }
        "oklch" => {
            let lch = color.to_oklch();
            Color::from_oklch(
                number("l").unwrap_or(lch.l),
                number("c").unwrap_or(lch.c),
                number("h").unwrap_or(lch.h),
                alpha.unwrap_or(lch.alpha),
            )
        }
        other => return Err(format!("unknown colour space `{other}`")),
    })
}

fn css(name: &str, values: [f64; 3], alpha: f64) -> String {
    let trim = |value: f64| {
        let text = format!("{value:.4}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    let body = values.map(trim).join(" ");
    if alpha < 1.0 {
        format!("{name}({body} / {})", trim(alpha))
    } else {
        format!("{name}({body})")
    }
}

/// The nearest of the 148 CSS names, by CIEDE2000.
pub(crate) fn nearest_name(color: &Color) -> &'static str {
    pastel::named::NAMED_COLORS
        .iter()
        .min_by(|a, b| {
            let da = color.distance_delta_e_ciede2000(&a.color);
            let db = color.distance_delta_e_ciede2000(&b.color);
            da.total_cmp(&db)
        })
        .map_or("black", |entry| entry.name)
}

pub(crate) fn install_color_methods<'gc>(ctx: Context<'gc>, methods: Table<'gc>) {
    macro_rules! method {
        ($name:literal, |$ctx:ident, $token:ident, $stack:ident| $body:block) => {
            let callback = Callback::from_fn(&ctx, |$ctx, _, mut $stack| {
                let $token: UserRef<ColorToken> = $stack.consume($ctx)?;
                let _ = &$token;
                $body
                Ok(CallbackReturn::Return)
            });
            methods.set_field(ctx, $name, callback);
        };
    }
    macro_rules! unary {
        ($name:literal, $op:expr) => {
            let callback = Callback::from_fn(&ctx, |ctx, _, mut stack| {
                let (token, amount): (UserRef<ColorToken>, f64) = stack.consume(ctx)?;
                let op: fn(&Color, f64) -> Color = $op;
                stack.replace(ctx, color_userdata(ctx, op(&token.color, amount)));
                Ok(CallbackReturn::Return)
            });
            methods.set_field(ctx, $name, callback);
        };
    }
    macro_rules! nullary {
        ($name:literal, $op:expr) => {
            let callback = Callback::from_fn(&ctx, |ctx, _, mut stack| {
                let token: UserRef<ColorToken> = stack.consume(ctx)?;
                let op: fn(&Color) -> Color = $op;
                stack.replace(ctx, color_userdata(ctx, op(&token.color)));
                Ok(CallbackReturn::Return)
            });
            methods.set_field(ctx, $name, callback);
        };
    }
    macro_rules! string {
        ($name:literal, $op:expr) => {
            let callback = Callback::from_fn(&ctx, |ctx, _, mut stack| {
                let token: UserRef<ColorToken> = stack.consume(ctx)?;
                let op: fn(&Color) -> String = $op;
                stack.replace(ctx, op(&token.color).as_str());
                Ok(CallbackReturn::Return)
            });
            methods.set_field(ctx, $name, callback);
        };
    }

    // Conversions, as tables.
    method!("rgb", |ctx, token, stack| {
        let c = token.color.to_rgba_float();
        stack.replace(
            ctx,
            table_of(ctx, &[("r", c.r), ("g", c.g), ("b", c.b), ("a", c.alpha)]),
        );
    });
    method!("rgb8", |ctx, token, stack| {
        let c = token.color.to_rgba();
        let table = Table::new(&ctx);
        table.set_field(ctx, "r", i64::from(c.r));
        table.set_field(ctx, "g", i64::from(c.g));
        table.set_field(ctx, "b", i64::from(c.b));
        table.set_field(ctx, "a", c.alpha);
        stack.replace(ctx, table);
    });
    method!("hsl", |ctx, token, stack| {
        let c = token.color.to_hsla();
        stack.replace(
            ctx,
            table_of(ctx, &[("h", c.h), ("s", c.s), ("l", c.l), ("a", c.alpha)]),
        );
    });
    method!("hsv", |ctx, token, stack| {
        let c = token.color.to_hsva();
        stack.replace(
            ctx,
            table_of(ctx, &[("h", c.h), ("s", c.s), ("v", c.v), ("a", c.alpha)]),
        );
    });
    method!("xyz", |ctx, token, stack| {
        let c = token.color.to_xyz();
        stack.replace(
            ctx,
            table_of(ctx, &[("x", c.x), ("y", c.y), ("z", c.z), ("a", c.alpha)]),
        );
    });
    method!("lms", |ctx, token, stack| {
        let c = token.color.to_lms();
        stack.replace(
            ctx,
            table_of(ctx, &[("l", c.l), ("m", c.m), ("s", c.s), ("a", c.alpha)]),
        );
    });
    method!("lab", |ctx, token, stack| {
        let c = token.color.to_lab();
        stack.replace(
            ctx,
            table_of(
                ctx,
                &[("l", c.l), ("a", c.a), ("b", c.b), ("alpha", c.alpha)],
            ),
        );
    });
    method!("oklab", |ctx, token, stack| {
        let c = token.color.to_oklab();
        stack.replace(
            ctx,
            table_of(
                ctx,
                &[("l", c.l), ("a", c.a), ("b", c.b), ("alpha", c.alpha)],
            ),
        );
    });
    method!("lch", |ctx, token, stack| {
        let c = token.color.to_lch();
        stack.replace(
            ctx,
            table_of(ctx, &[("l", c.l), ("c", c.c), ("h", c.h), ("a", c.alpha)]),
        );
    });
    method!("oklch", |ctx, token, stack| {
        let c = token.color.to_oklch();
        stack.replace(
            ctx,
            table_of(ctx, &[("l", c.l), ("c", c.c), ("h", c.h), ("a", c.alpha)]),
        );
    });
    method!("cmyk", |ctx, token, stack| {
        let c = token.color.to_cmyk();
        stack.replace(
            ctx,
            table_of(ctx, &[("c", c.c), ("m", c.m), ("y", c.y), ("k", c.k)]),
        );
    });

    // Strings, in every format pastel writes.
    string!("hex", |c| c.to_rgb_hex_string(true));
    string!("rgb_string", |c| c.to_rgb_string(Format::Spaces));
    string!("rgb_float_string", |c| c
        .to_rgb_float_string(Format::Spaces));
    string!("hsl_string", |c| c.to_hsl_string(Format::Spaces));
    string!("hsv_string", |c| c.to_hsv_string(Format::Spaces));
    // The CIE and Ok spaces in the CSS form the parser reads back.
    string!("lab_string", |c| {
        let v = c.to_lab();
        css("lab", [v.l, v.a, v.b], v.alpha)
    });
    string!("lch_string", |c| {
        let v = c.to_lch();
        css("lch", [v.l, v.c, v.h], v.alpha)
    });
    string!("oklab_string", |c| {
        let v = c.to_oklab();
        css("oklab", [v.l, v.a, v.b], v.alpha)
    });
    string!("oklch_string", |c| {
        let v = c.to_oklch();
        css("oklch", [v.l, v.c, v.h], v.alpha)
    });
    string!("cmyk_string", |c| c.to_cmyk_string(Format::Spaces));
    string!("nearest_name", |c| nearest_name(c).to_owned());

    // Changes.
    unary!("lighten", |c, amount| c.lighten(amount));
    unary!("darken", |c, amount| c.darken(amount));
    unary!("saturate", |c, amount| c.saturate(amount));
    unary!("desaturate", |c, amount| c.desaturate(amount));
    unary!("rotate", |c, degrees| c.rotate_hue(degrees));
    unary!("alpha", with_alpha);
    nullary!("complement", |c| c.complementary());
    nullary!("gray", |c| c.to_gray());
    nullary!("invert", |c| {
        let rgba = c.to_rgba_float();
        Color::from_rgba_float(1.0 - rgba.r, 1.0 - rgba.g, 1.0 - rgba.b, rgba.alpha)
    });
    let mix = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, other, fraction, space): (
            UserRef<ColorToken>,
            LuaValue,
            Option<f64>,
            Option<String>,
        ) = stack.consume(ctx)?;
        let other = color_of(ctx, other).map_err(HostError)?;
        let mixed = mix_in(
            &token.color,
            &other,
            fraction.unwrap_or(0.5),
            space.as_deref(),
        )
        .map_err(HostError)?;
        stack.replace(ctx, color_userdata(ctx, mixed));
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "mix", mix);
    let composite = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, over): (UserRef<ColorToken>, LuaValue) = stack.consume(ctx)?;
        let over = color_of(ctx, over).map_err(HostError)?;
        // `pastel` composites its argument over the receiver; this colour
        // goes over the one given.
        stack.replace(ctx, color_userdata(ctx, over.composite(&token.color)));
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "composite", composite);
    let blind = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, kind): (UserRef<ColorToken>, String) = stack.consume(ctx)?;
        let kind = match kind.as_str() {
            "protanopia" => pastel::ColorblindnessType::Protanopia,
            "deuteranopia" => pastel::ColorblindnessType::Deuteranopia,
            "tritanopia" => pastel::ColorblindnessType::Tritanopia,
            other => {
                return Err(HostError(format!(
                    "unknown colour blindness `{other}`: protanopia, deuteranopia or tritanopia"
                ))
                .into());
            }
        };
        stack.replace(
            ctx,
            color_userdata(ctx, token.color.simulate_colorblindness(kind)),
        );
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "blind", blind);
    let with = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, changes): (UserRef<ColorToken>, Table) = stack.consume(ctx)?;
        let changed = with_channels(ctx, &token.color, changes).map_err(HostError)?;
        stack.replace(ctx, color_userdata(ctx, changed));
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "with", with);

    // Measures.
    method!("luminance", |ctx, token, stack| {
        stack.replace(ctx, token.color.luminance());
    });
    method!("brightness", |ctx, token, stack| {
        stack.replace(ctx, token.color.brightness());
    });
    method!("is_light", |ctx, token, stack| {
        stack.replace(ctx, token.color.is_light());
    });
    nullary!("text_color", |c| c.text_color());
    let contrast = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, other): (UserRef<ColorToken>, LuaValue) = stack.consume(ctx)?;
        let other = color_of(ctx, other).map_err(HostError)?;
        stack.replace(ctx, token.color.contrast_ratio(&other));
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "contrast", contrast);
    let distance = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, other, metric): (UserRef<ColorToken>, LuaValue, Option<String>) =
            stack.consume(ctx)?;
        let other = color_of(ctx, other).map_err(HostError)?;
        let value = match metric.as_deref().unwrap_or("ciede2000") {
            "cie76" => token.color.distance_delta_e_cie76(&other),
            "ciede2000" => token.color.distance_delta_e_ciede2000(&other),
            other => {
                return Err(
                    HostError(format!("unknown distance `{other}`: cie76 or ciede2000")).into(),
                );
            }
        };
        stack.replace(ctx, value);
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "distance", distance);
}
