//! Between colours: scales, distinct palettes, random colours, terminals.

use luna::{Callback, CallbackReturn, Context, Table, UserData, UserRef, Value as LuaValue};
use pastel::ansi::{AnsiColor, Brush, Mode, Style, ToAnsiStyle};
use pastel::distinct::{
    DistanceMetric, OptimizationMode, OptimizationTarget, SimulatedAnnealing, SimulationParameters,
    rearrange_sequence,
};
use pastel::random::{RandomizationStrategy, strategies};
use pastel::{Color, ColorScale, Fraction};

use crate::api_color::{ColorToken, color_of, color_userdata};
use crate::api_color_ops::mix_in;
use crate::scene_bindings::*;

/// A ramp of colours a fraction is read from.
pub(crate) struct ScaleToken {
    stops: Vec<(f64, Color)>,
}

fn sample(stops: &[(f64, Color)], at: f64, space: Option<&str>) -> Result<Option<Color>, String> {
    let mut scale = ColorScale::empty();
    for (position, color) in stops {
        scale.add_stop(color.clone(), Fraction::from(*position));
    }
    let space = space.map(str::to_owned);
    let mix = move |a: &Color, b: &Color, fraction: Fraction| {
        mix_in(a, b, fraction.value(), space.as_deref()).unwrap_or_else(|_| a.clone())
    };
    Ok(scale.sample(Fraction::from(at), &mix))
}

fn metric_of(name: Option<&str>) -> Result<DistanceMetric, String> {
    Ok(match name.unwrap_or("ciede2000") {
        "cie76" => DistanceMetric::CIE76,
        "ciede2000" => DistanceMetric::CIEDE2000,
        other => return Err(format!("unknown distance `{other}`: cie76 or ciede2000")),
    })
}

pub(crate) fn install_palette_constructors<'gc>(ctx: Context<'gc>, color: Table<'gc>) {
    // `morf.color.scale { { color, position }, ... }` or a bare list of
    // colours spread evenly.
    let sample_callback = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (scale, at, space): (UserRef<ScaleToken>, f64, Option<String>) = stack.consume(ctx)?;
        let value = sample(&scale.stops, at, space.as_deref()).map_err(HostError)?;
        match value {
            Some(color) => stack.replace(ctx, color_userdata(ctx, color)),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let samples_callback = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (scale, count, space): (UserRef<ScaleToken>, i64, Option<String>) =
            stack.consume(ctx)?;
        let count = count.max(1) as usize;
        let table = Table::new(&ctx);
        for index in 0..count {
            let at = if count == 1 {
                0.0
            } else {
                index as f64 / (count - 1) as f64
            };
            let value = sample(&scale.stops, at, space.as_deref()).map_err(HostError)?;
            table
                .set(
                    ctx,
                    index as i64 + 1,
                    value.map_or(LuaValue::Nil, |c| color_userdata(ctx, c)),
                )
                .map_err(|error| HostError(error.to_string()))?;
        }
        stack.replace(ctx, table);
        Ok(CallbackReturn::Return)
    });
    let scale_methods = Table::new(&ctx);
    scale_methods.set_field(ctx, "sample", sample_callback);
    scale_methods.set_field(ctx, "samples", samples_callback);
    let scale_metatable = Table::new(&ctx);
    scale_metatable.set_field(ctx, "__index", scale_methods);
    let scale_metatable = ctx.stash(scale_metatable);
    let scale = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let stops: Table = stack.consume(ctx)?;
        let entries = stops.iter(ctx).collect::<Vec<_>>();
        let count = entries.len();
        let mut parsed = Vec::with_capacity(count);
        for (index, (_, entry)) in entries.into_iter().enumerate() {
            let even = if count <= 1 {
                0.0
            } else {
                index as f64 / (count - 1) as f64
            };
            match entry {
                LuaValue::Table(pair) => {
                    let color = color_of(ctx, pair.get_value(ctx, 1)).map_err(HostError)?;
                    let position = match pair.get_value(ctx, 2) {
                        LuaValue::Integer(value) => value as f64,
                        LuaValue::Number(value) => value,
                        _ => even,
                    };
                    parsed.push((position, color));
                }
                other => parsed.push((even, color_of(ctx, other).map_err(HostError)?)),
            }
        }
        parsed.sort_by(|a, b| a.0.total_cmp(&b.0));
        let userdata = UserData::new_static(&ctx, ScaleToken { stops: parsed });
        userdata.set_metatable(ctx, Some(ctx.fetch(&scale_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "scale", scale);

    // `morf.color.distinct(n, { fixed = {...}, metric = "ciede2000", order = true,
    // iterations = 300000 })`: `n` colours as far apart as a search can
    // put them, `fixed` ones kept, `order`ed so neighbours differ most.
    let distinct = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (count, options): (i64, Option<Table>) = stack.consume(ctx)?;
        if count < 2 {
            return Err(HostError("distinct colours need a count of at least two".into()).into());
        }
        let mut fixed = Vec::new();
        let mut metric = None;
        let mut order = false;
        let mut iterations = 300_000usize;
        if let Some(options) = options {
            if let LuaValue::Table(list) = options.get_value(ctx, "fixed") {
                for (_, value) in list.iter(ctx) {
                    fixed.push(color_of(ctx, value).map_err(HostError)?);
                }
            }
            if let LuaValue::String(name) = options.get_value(ctx, "metric") {
                metric = Some(name.display_lossy().to_string());
            }
            order = matches!(options.get_value(ctx, "order"), LuaValue::Boolean(true));
            if let LuaValue::Integer(value) = options.get_value(ctx, "iterations") {
                iterations = usize::try_from(value.max(1)).unwrap_or(1);
            }
        }
        let metric = metric_of(metric.as_deref()).map_err(HostError)?;
        if fixed.len() > count as usize {
            return Err(HostError("more fixed colours than the count asked for".into()).into());
        }
        let mut colors = distinct(count as usize, metric, fixed, iterations);
        if order {
            rearrange_sequence(&mut colors, metric);
        }
        let table = Table::new(&ctx);
        for (index, color) in colors.into_iter().enumerate() {
            table
                .set(ctx, index as i64 + 1, color_userdata(ctx, color))
                .map_err(|error| HostError(error.to_string()))?;
        }
        stack.replace(ctx, table);
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "distinct", distinct);

    let random = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let kind: Option<String> = stack.consume(ctx)?;
        let color = match kind.as_deref().unwrap_or("vivid") {
            "vivid" => strategies::Vivid.generate(),
            "rgb" => strategies::UniformRGB.generate(),
            "gray" => strategies::UniformGray.generate(),
            "hue" => strategies::UniformHueLCh.generate(),
            other => {
                return Err(HostError(format!(
                    "unknown random kind `{other}`: vivid, rgb, gray or hue"
                ))
                .into());
            }
        };
        stack.replace(ctx, color_userdata(ctx, color));
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "random", random);

    let ansi8 = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let index: i64 = stack.consume(ctx)?;
        let index = u8::try_from(index).map_err(|_| HostError("an ANSI index is 0..255".into()))?;
        stack.replace(ctx, color_userdata(ctx, Color::from_ansi_8bit(index)));
        Ok(CallbackReturn::Return)
    });
    color.set_field(ctx, "ansi8", ansi8);
}

/// `pastel`'s search, with the iteration budget in the caller's hands: a
/// global pass over a third of it, a local pass over the rest.
fn distinct(
    count: usize,
    metric: DistanceMetric,
    fixed: Vec<Color>,
    iterations: usize,
) -> Vec<Color> {
    let num_fixed_colors = fixed.len();
    let mut colors = fixed;
    for _ in num_fixed_colors..count {
        colors.push(strategies::UniformRGB.generate());
    }
    let mut annealing = SimulatedAnnealing::new(
        &colors,
        SimulationParameters {
            initial_temperature: 3.0,
            cooling_rate: 0.95,
            num_iterations: iterations / 3,
            opt_target: OptimizationTarget::Mean,
            opt_mode: OptimizationMode::Global,
            distance_metric: metric,
            num_fixed_colors,
        },
    );
    annealing.run(&mut |_| {});
    annealing.parameters.initial_temperature = 0.5;
    annealing.parameters.cooling_rate = 0.98;
    annealing.parameters.num_iterations = iterations - iterations / 3;
    annealing.parameters.opt_target = OptimizationTarget::Min;
    annealing.parameters.opt_mode = OptimizationMode::Local;
    annealing.run(&mut |_| {});
    annealing.get_colors()
}

pub(crate) fn install_palette_methods<'gc>(ctx: Context<'gc>, methods: Table<'gc>) {
    let ansi8 = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let token: UserRef<ColorToken> = stack.consume(ctx)?;
        stack.replace(ctx, i64::from(token.color.to_ansi_8bit()));
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "ansi8", ansi8);
    let sequence = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, mode): (UserRef<ColorToken>, Option<String>) = stack.consume(ctx)?;
        let mode = match mode.as_deref().unwrap_or("truecolor") {
            "truecolor" => Mode::TrueColor,
            "8bit" => Mode::Ansi8Bit,
            other => {
                return Err(
                    HostError(format!("unknown ANSI mode `{other}`: truecolor or 8bit")).into(),
                );
            }
        };
        stack.replace(ctx, token.color.to_ansi_sequence(mode).as_str());
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "ansi_sequence", sequence);
    // `c:ansi_style { bold = true, on = other, mode = "8bit" }` gives the
    // escape sequence that styles text this colour; `c:paint(text, style)`
    // wraps text in it and resets after.
    let style = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, options): (UserRef<ColorToken>, Option<Table>) = stack.consume(ctx)?;
        let (style, mode) = style_of(ctx, &token.color, options).map_err(HostError)?;
        stack.replace(ctx, style.escape_sequence(mode).as_str());
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "ansi_style", style);
    let paint = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (token, text, options): (UserRef<ColorToken>, String, Option<Table>) =
            stack.consume(ctx)?;
        let (style, mode) = style_of(ctx, &token.color, options).map_err(HostError)?;
        stack.replace(
            ctx,
            Brush::from_mode(Some(mode)).paint(text, style).as_str(),
        );
        Ok(CallbackReturn::Return)
    });
    methods.set_field(ctx, "paint", paint);
}

fn style_of<'gc>(
    ctx: Context<'gc>,
    color: &Color,
    options: Option<Table<'gc>>,
) -> Result<(Style, Mode), String> {
    let mut style = color.ansi_style();
    let mut mode = Mode::TrueColor;
    if let Some(options) = options {
        let flag = |key: &str| matches!(options.get_value(ctx, key), LuaValue::Boolean(true));
        if flag("bold") {
            style.bold(true);
        }
        if flag("italic") {
            style.italic(true);
        }
        if flag("underline") {
            style.underline(true);
        }
        match options.get_value(ctx, "on") {
            LuaValue::Nil => {}
            value => {
                style.on(color_of(ctx, value)?);
            }
        }
        if let LuaValue::String(name) = options.get_value(ctx, "mode") {
            mode = match name.display_lossy().to_string().as_str() {
                "truecolor" => Mode::TrueColor,
                "8bit" => Mode::Ansi8Bit,
                other => return Err(format!("unknown ANSI mode `{other}`")),
            };
        }
    }
    Ok((style, mode))
}
