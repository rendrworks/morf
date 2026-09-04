//! `morf.color`: a colour as a value.
//!
//! What a configuration holds is `pastel`'s colour, which converts to and
//! from every space the crate knows and answers every question the
//! `pastel` command line can. It is accepted wherever a colour property is
//! written, handed back wherever one is read, and may sit in a signal or a
//! `morf.state` field. `tostring` gives the hex, so it drops into a string
//! anywhere one was expected before.

use luna::{
    Callback, CallbackReturn, Context, Table, UserData, UserRef, Value as LuaValue, Variadic,
};
use morf_scene::Color as SceneColor;
use pastel::Color;

use crate::{api_color_ops::install_color_methods, scene_bindings::*};

/// The colour value a configuration holds.
pub(crate) struct ColorToken {
    pub(crate) color: Color,
}

/// The global the colour metatable lives under, so a colour can be built
/// from anywhere a scene value is turned into a Lua one.
const METATABLE_GLOBAL: &str = "__morf_color";

/// Wraps a colour as a Lua value.
pub(crate) fn color_userdata<'gc>(ctx: Context<'gc>, color: Color) -> LuaValue<'gc> {
    let userdata = UserData::new_static(&ctx, ColorToken { color });
    if let Ok(metatable) = ctx.get_global::<Table>(METATABLE_GLOBAL) {
        userdata.set_metatable(ctx, Some(metatable));
    }
    LuaValue::UserData(userdata)
}

/// A scene colour as a Lua value.
pub(crate) fn scene_color_userdata<'gc>(ctx: Context<'gc>, color: SceneColor) -> LuaValue<'gc> {
    color_userdata(ctx, color.to_pastel())
}

/// Reads a colour from anything a configuration may hand over: a colour
/// value, a string in any syntax, or a table naming a space.
pub(crate) fn color_of<'gc>(ctx: Context<'gc>, value: LuaValue<'gc>) -> Result<Color, String> {
    match value {
        LuaValue::UserData(userdata) => userdata
            .downcast_static::<ColorToken>()
            .map(|token| token.color.clone())
            .map_err(|_| "expected a colour".to_owned()),
        LuaValue::String(text) => {
            let text = text.display_lossy().to_string();
            SceneColor::parse(&text)
                .map(|color| color.to_pastel())
                .ok_or_else(|| format!("`{text}` is not a colour"))
        }
        LuaValue::Table(table) => color_from_table(ctx, table),
        other => Err(format!("expected a colour, not {}", other.type_name())),
    }
}

/// `{ r, g, b, a }`, `{ h, s, l }`, `{ h, s, v }`, `{ l, a, b }`, or any of
/// those with `space = "..."` naming which.
fn color_from_table<'gc>(ctx: Context<'gc>, table: Table<'gc>) -> Result<Color, String> {
    let number = |key: &str| match table.get_value(ctx, key) {
        LuaValue::Integer(value) => Some(value as f64),
        LuaValue::Number(value) if value.is_finite() => Some(value),
        _ => None,
    };
    let alpha = number("a").or_else(|| number("alpha")).unwrap_or(1.0);
    let space = match table.get_value(ctx, "space") {
        LuaValue::String(name) => name.display_lossy().to_string(),
        _ => {
            if number("r").is_some() {
                "rgb".to_owned()
            } else if number("v").is_some() {
                "hsv".to_owned()
            } else if number("s").is_some() {
                "hsl".to_owned()
            } else if number("c").is_some() {
                "oklch".to_owned()
            } else if number("l").is_some() {
                "oklab".to_owned()
            } else {
                return Err("a colour table needs r, g, b or h, s, l or l, a, b".to_owned());
            }
        }
    };
    let need = |key: &str| number(key).ok_or_else(|| format!("colour table lacks `{key}`"));
    Ok(match space.as_str() {
        "rgb" => {
            let (r, g, b) = (need("r")?, need("g")?, need("b")?);
            if r > 1.0 || g > 1.0 || b > 1.0 {
                Color::from_rgba(r as u8, g as u8, b as u8, alpha)
            } else {
                Color::from_rgba_float(r, g, b, alpha)
            }
        }
        "hsl" => Color::from_hsla(need("h")?, need("s")?, need("l")?, alpha),
        "hsv" => Color::from_hsva(need("h")?, need("s")?, need("v")?, alpha),
        "lab" => Color::from_lab(need("l")?, need("a")?, need("b")?, alpha),
        "oklab" => Color::from_oklab(need("l")?, need("a")?, need("b")?, alpha),
        "lch" => Color::from_lch(need("l")?, need("c")?, need("h")?, alpha),
        "oklch" => Color::from_oklch(need("l")?, need("c")?, need("h")?, alpha),
        "xyz" => Color::from_xyz(need("x")?, need("y")?, need("z")?, alpha),
        "lms" => Color::from_lms(need("l")?, need("m")?, need("s")?, alpha),
        "cmyk" => from_cmyk(need("c")?, need("m")?, need("y")?, need("k")?),
        other => return Err(format!("unknown colour space `{other}`")),
    })
}

/// Ink fractions in 0..1 to a colour. (`pastel`'s own conversion divides by
/// a hundred where it should not, so it is not used.)
fn from_cmyk(c: f64, m: f64, y: f64, k: f64) -> Color {
    let ink = |value: f64| (1.0 - value.clamp(0.0, 1.0)) * (1.0 - k.clamp(0.0, 1.0));
    Color::from_rgba_float(ink(c), ink(m), ink(y), 1.0)
}

fn number_arg(value: LuaValue<'_>, what: &str) -> Result<f64, String> {
    match value {
        LuaValue::Integer(value) => Ok(value as f64),
        LuaValue::Number(value) if value.is_finite() => Ok(value),
        _ => Err(format!("{what} must be a number")),
    }
}

/// Reads three or four numbers, the fourth an alpha that defaults to one.
fn channels<'gc>(
    values: &[LuaValue<'gc>],
    names: [&str; 3],
) -> Result<(f64, f64, f64, f64), String> {
    let read = |index: usize, name: &str| -> Result<f64, String> {
        number_arg(values.get(index).copied().unwrap_or(LuaValue::Nil), name)
    };
    let a = read(0, names[0])?;
    let b = read(1, names[1])?;
    let c = read(2, names[2])?;
    let alpha = match values.get(3) {
        None | Some(LuaValue::Nil) => 1.0,
        Some(value) => number_arg(*value, "alpha")?,
    };
    Ok((a, b, c, alpha))
}

pub(crate) fn install_color_api<'gc>(ctx: Context<'gc>, morf: Table<'gc>) {
    let methods = Table::new(&ctx);
    install_color_methods(ctx, methods);
    crate::api_color_palette::install_palette_methods(ctx, methods);
    let methods = ctx.stash(methods);

    // Fields first -- `c.r`, `c.h` -- then the method table.
    let index = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (token, key): (UserRef<ColorToken>, String) = stack.consume(ctx)?;
        let value = match key.as_str() {
            "r" | "g" | "b" | "a" => {
                let rgba = token.color.to_rgba_float();
                Some(match key.as_str() {
                    "r" => rgba.r,
                    "g" => rgba.g,
                    "b" => rgba.b,
                    _ => rgba.alpha,
                })
            }
            "h" | "s" | "l" => {
                let hsla = token.color.to_hsla();
                Some(match key.as_str() {
                    "h" => hsla.h,
                    "s" => hsla.s,
                    _ => hsla.l,
                })
            }
            _ => None,
        };
        match value {
            Some(number) => stack.replace(ctx, number),
            None => stack.replace(ctx, ctx.fetch(&methods).get_value(ctx, key.as_str())),
        }
        Ok(CallbackReturn::Return)
    });
    let to_string = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let token: UserRef<ColorToken> = stack.consume(ctx)?;
        stack.replace(ctx, token.color.to_rgb_hex_string(true).as_str());
        Ok(CallbackReturn::Return)
    });
    let equals = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (a, b): (LuaValue, LuaValue) = stack.consume(ctx)?;
        let same = match (color_of(ctx, a), color_of(ctx, b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        stack.replace(ctx, same);
        Ok(CallbackReturn::Return)
    });
    let metatable = Table::new(&ctx);
    metatable.set_field(ctx, "__index", index);
    metatable.set_field(ctx, "__tostring", to_string);
    metatable.set_field(ctx, "__eq", equals);
    metatable.set_field(ctx, "__name", "color");
    ctx.set_global(METATABLE_GLOBAL, metatable);

    let color = Table::new(&ctx);
    // `morf.color(x)`: from a string, a table or another colour.
    let call = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (_, value): (Table, LuaValue) = stack.consume(ctx)?;
        let parsed = color_of(ctx, value).map_err(HostError)?;
        stack.replace(ctx, color_userdata(ctx, parsed));
        Ok(CallbackReturn::Return)
    });
    let callable = Table::new(&ctx);
    callable.set_field(ctx, "__call", call);
    color.set_metatable(ctx, Some(callable));

    for (name, space) in [
        ("rgb", "rgb"),
        ("hsl", "hsl"),
        ("hsv", "hsv"),
        ("lab", "lab"),
        ("oklab", "oklab"),
        ("lch", "lch"),
        ("oklch", "oklch"),
        ("xyz", "xyz"),
        ("lms", "lms"),
    ] {
        let constructor = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let values: Variadic<Vec<LuaValue>> = stack.consume(ctx)?;
            let names = match space {
                "rgb" => ["r", "g", "b"],
                "hsl" => ["h", "s", "l"],
                "hsv" => ["h", "s", "v"],
                "lab" | "oklab" => ["l", "a", "b"],
                "lch" | "oklch" => ["l", "c", "h"],
                "xyz" => ["x", "y", "z"],
                _ => ["l", "m", "s"],
            };
            let (a, b, c, alpha) = channels(&values, names).map_err(HostError)?;
            let color = match space {
                "rgb" => {
                    if a > 1.0 || b > 1.0 || c > 1.0 {
                        Color::from_rgba(a as u8, b as u8, c as u8, alpha)
                    } else {
                        Color::from_rgba_float(a, b, c, alpha)
                    }
                }
                "hsl" => Color::from_hsla(a, b, c, alpha),
                "hsv" => Color::from_hsva(a, b, c, alpha),
                "lab" => Color::from_lab(a, b, c, alpha),
                "oklab" => Color::from_oklab(a, b, c, alpha),
                "lch" => Color::from_lch(a, b, c, alpha),
                "oklch" => Color::from_oklch(a, b, c, alpha),
                "xyz" => Color::from_xyz(a, b, c, alpha),
                _ => Color::from_lms(a, b, c, alpha),
            };
            stack.replace(ctx, color_userdata(ctx, color));
            Ok(CallbackReturn::Return)
        });
        color.set_field(ctx, name, constructor);
    }
    let cmyk = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (c, m, y, k): (f64, f64, f64, f64) = stack.consume(ctx)?;
        stack.replace(ctx, color_userdata(ctx, from_cmyk(c, m, y, k)));
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "cmyk", cmyk);
    let gray = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (lightness, alpha): (f64, Option<f64>) = stack.consume(ctx)?;
        let mut color = Color::graytone(lightness);
        if let Some(alpha) = alpha {
            let hsla = color.to_hsla();
            color = Color::from_hsla(hsla.h, hsla.s, hsla.l, alpha);
        }
        stack.replace(ctx, color_userdata(ctx, color));
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "gray", gray);
    let named = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let name: String = stack.consume(ctx)?;
        let found = pastel::named::NAMED_COLORS
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(&name))
            .ok_or_else(|| HostError(format!("no colour is named `{name}`")))?;
        stack.replace(ctx, color_userdata(ctx, found.color.clone()));
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "named", named);
    let names = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let table = Table::new(&ctx);
        for entry in pastel::named::NAMED_COLORS.iter() {
            table.set_field(ctx, entry.name, color_userdata(ctx, entry.color.clone()));
        }
        stack.replace(ctx, table);
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "names", names);
    let mix = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (a, b, fraction, space): (LuaValue, LuaValue, Option<f64>, Option<String>) =
            stack.consume(ctx)?;
        let a = color_of(ctx, a).map_err(HostError)?;
        let b = color_of(ctx, b).map_err(HostError)?;
        let mixed = crate::api_color_ops::mix_in(&a, &b, fraction.unwrap_or(0.5), space.as_deref())
            .map_err(HostError)?;
        stack.replace(ctx, color_userdata(ctx, mixed));
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "mix", mix);
    crate::api_color_palette::install_palette_constructors(ctx, color);
    morf.set_field(ctx, "color", color);
}
