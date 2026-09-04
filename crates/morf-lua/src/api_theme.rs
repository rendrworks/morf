//! `morf.theme(tokens, options)`: a state of named appearance tokens.
//!
//! A theme is a `morf.state` with three things a plain state does not have.
//! A string that names a colour becomes one, because in a theme that is what
//! it is. A function field is derived: read through the proxy it runs with
//! the theme as its argument, and read inside a binding whatever it touches
//! is what the binding tracks, so `hover = function(t) return t.accent:alpha(0.5) end`
//! follows `accent` with no wiring. And a `source` is a JSON file whose leaf
//! keys are tokens — the file a palette generator writes — read now and
//! again whenever it is rewritten.

use luna::{Callback, CallbackReturn, Context, Function, Table, Value as LuaValue};
use morf_io::FileView;
use morf_reactive::SignalId;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::{api_state::build, scene_bindings::*, state::*, surface_types::*, types::*};

/// Whether text is written the way a colour is: `#` or `0x` hex, a
/// functional form, or a bare name. Digits alone are a number someone
/// wrote as a string, not a hex colour, however happily a parser reads them.
fn written_as_color(text: &str) -> bool {
    text.starts_with('#')
        || text.starts_with("0x")
        || text.contains('(')
        || (!text.is_empty()
            && text
                .chars()
                .all(|character| character.is_ascii_alphabetic()))
}

/// A token as a theme keeps it: a colour where the text names one.
pub(crate) fn token_value(value: IpcValue) -> IpcValue {
    match value {
        IpcValue::String(text) if written_as_color(&text) => {
            match morf_scene::Color::parse(&text) {
                Some(color) => IpcValue::Color(color),
                None => IpcValue::String(text),
            }
        }
        other => other,
    }
}

/// `~` at the front of a path is the home directory.
fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => Path::new(&home).join(rest),
            None => PathBuf::from(path),
        },
        None => PathBuf::from(path),
    }
}

fn collect_tokens(value: &serde_json::Value, into: &mut BTreeMap<String, IpcValue>) {
    let serde_json::Value::Object(entries) = value else {
        return;
    };
    for (key, value) in entries {
        let token = match value {
            serde_json::Value::Object(_) => {
                collect_tokens(value, into);
                continue;
            }
            serde_json::Value::Array(_) | serde_json::Value::Null => continue,
            serde_json::Value::Bool(value) => IpcValue::Boolean(*value),
            serde_json::Value::Number(number) => match number.as_i64() {
                Some(value) => IpcValue::Integer(value),
                None => IpcValue::Number(number.as_f64().unwrap_or(0.0)),
            },
            serde_json::Value::String(text) => token_value(IpcValue::String(text.clone())),
        };
        into.insert(key.clone(), token);
    }
}

/// The tokens a JSON file holds: every leaf under an object, named by its
/// own key. Nested objects are walked, so `colors.color1` is `color1`;
/// arrays are not tokens.
pub(crate) fn read_tokens(path: &Path) -> Result<BTreeMap<String, IpcValue>, String> {
    let bytes = FileView::new(path)
        .read_bounded(1024 * 1024)
        .map_err(|error| format!("{}: {}", path.display(), error.as_str()))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut tokens = BTreeMap::new();
    collect_tokens(&value, &mut tokens);
    Ok(tokens)
}

pub(crate) fn install_theme_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
    _limits: Limits,
) {
    let theme = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (seed, options): (Table, Option<Table>) = stack.consume(ctx)?;
            let option = |name: &str| options.map(|options| options.get_value(ctx, name));
            let reloadable = match option("reloadable") {
                Some(LuaValue::String(name)) => Some(name.display_lossy().to_string()),
                Some(LuaValue::Nil) | None => None,
                Some(_) => return Err(HostError("theme `reloadable` is a name".into()).into()),
            };
            let source = match option("source") {
                Some(LuaValue::String(path)) => {
                    Some(expand_home(&path.display_lossy().to_string()))
                }
                Some(LuaValue::Nil) | None => None,
                Some(_) => return Err(HostError("theme `source` is a path".into()).into()),
            };
            // Functions are derived and stay out of the state; everything else
            // is a token, colour-named strings included.
            let merged = Table::new(&ctx);
            let mut derived = HashMap::new();
            for (key, value) in seed.iter(ctx) {
                let LuaValue::String(name) = key else {
                    return Err(HostError("theme tokens are named".to_owned()).into());
                };
                match value {
                    LuaValue::Function(Function::Closure(closure)) => {
                        derived.insert(name.display_lossy().to_string(), ctx.stash(closure));
                    }
                    LuaValue::Function(_) => {
                        return Err(HostError(format!(
                            "theme token `{}` must be a value or a Lua function",
                            name.display_lossy()
                        ))
                        .into());
                    }
                    LuaValue::String(text) => {
                        let token = token_value(IpcValue::String(text.display_lossy().to_string()));
                        merged
                            .set(ctx, key, token.to_lua(ctx))
                            .map_err(|error| HostError(error.to_string()))?;
                    }
                    other => {
                        merged
                            .set(ctx, key, other)
                            .map_err(|error| HostError(error.to_string()))?;
                    }
                }
            }
            // The file is live data, so what it says wins over the seed; a
            // missing file leaves the seed as it is.
            let mut source_keys = Vec::new();
            if let Some(path) = &source
                && path.exists()
            {
                for (key, value) in read_tokens(path).map_err(HostError)? {
                    merged
                        .set(ctx, key.as_str(), value.to_lua(ctx))
                        .map_err(|error| HostError(error.to_string()))?;
                    source_keys.push(key);
                }
            }
            let metatable = state
                .borrow()
                .state_metatable
                .clone()
                .ok_or_else(|| HostError("states are not installed".to_owned()))?;
            let userdata = build(
                ctx,
                &state,
                &metatable,
                "theme",
                reloadable.as_deref(),
                merged,
            )
            .map_err(HostError)?;
            let token = userdata
                .downcast_static::<StateToken>()
                .map_err(|_| HostError("theme is not a state".to_owned()))?;
            let mut fields = token.fields.borrow_mut();
            fields.derived = derived;
            fields.theme = true;
            if let Some(path) = source {
                let watched: HashMap<String, SignalId> = source_keys
                    .iter()
                    .filter_map(|key| Some((key.clone(), *fields.scalars.get(key)?)))
                    .collect();
                let watcher = FileView::new(&path).watch().ok();
                state.borrow_mut().theme_sources.push(ThemeSource {
                    path,
                    watcher,
                    fields: watched,
                });
            }
            drop(fields);
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });
    morf.set_field(ctx, "theme", theme);
}
