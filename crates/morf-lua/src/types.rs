use luna::Lua;
use std::cell::RefCell;
use std::error::Error as StdError;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::state::*;

/// Execution limits applied independently to each loaded chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Maximum VM fuel a chunk may consume.
    pub fuel: u64,
    /// Maximum bytes owned by the Lua state.
    pub memory: usize,
    /// VM fuel granted before the host regains control.
    pub slice_fuel: i32,
    /// Maximum VM fuel granted to one reactive Lua effect.
    pub effect_fuel: u64,
    /// Maximum VM fuel granted to all effects in one recompute pass.
    pub frame_fuel: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 10_000_000,
            memory: 64 * 1024 * 1024,
            slice_fuel: 4_096,
            effect_fuel: 100_000,
            frame_fuel: 1_000_000,
        }
    }
}

/// A configuration execution failure.
#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    /// The source could not be compiled.
    Load(String),
    /// Execution stopped with a Lua error.
    Runtime(String),
    /// Execution exceeded its instruction budget.
    FuelExhausted { budget: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(message) => write!(f, "could not load Lua: {message}"),
            Self::Runtime(message) => write!(f, "Lua error: {message}"),
            Self::FuelExhausted { budget } => {
                write!(f, "Lua fuel exhausted after {budget} instructions")
            }
        }
    }
}

impl StdError for Error {}

/// The Luna VM owned behind morf's stable runtime boundary.
pub struct Runtime {
    pub(crate) lua: Lua,
    pub(crate) limits: Limits,
    pub(crate) reactive: Rc<RefCell<ReactiveState>>,
    pub(crate) module_roots: Rc<RefCell<Vec<PathBuf>>>,
}

/// How much a log line matters.
///
/// Ordered, so a filter is a comparison: asking for warnings gets errors too.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LogLevel {
    /// Something a person tuning a configuration wants and nobody else does.
    Debug,
    /// What happened, when it is worth saying.
    Info,
    /// Something went wrong and the shell carried on.
    Warn,
    /// Something went wrong and did not.
    Error,
}

impl LogLevel {
    /// The name on the wire and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// Reads a level back, for `--level` and for the wire.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// One line of the shell's log.
///
/// Carries when and how much rather than only what. A flat list of strings is
/// unreadable by the time it matters: a shell that has been running for a day
/// has thousands of them and no way to ask which are serious, or recent.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub level: LogLevel,
    /// Milliseconds since the epoch. A number rather than a formatted time, so
    /// whoever shows it decides how.
    pub at_ms: u64,
    pub message: String,
}

impl std::fmt::Display for LogEntry {
    /// Level and message, which is what a person reading one wants.
    ///
    /// Not the timestamp: it is a number of milliseconds and whoever shows it
    /// decides how, which is the whole reason it is stored as one.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:<5} {}", self.level.name(), self.message)
    }
}

impl LogEntry {
    /// Packs an entry into one wire string.
    ///
    /// Unit separators, because they cannot occur in a log message and so need
    /// no escaping -- and a log format that needs escaping gets it wrong on the
    /// one line you most wanted to read.
    pub fn to_wire(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.level.name(),
            self.at_ms,
            self.message
        )
    }

    /// Reads one back, tolerating a line that was never packed.
    pub fn from_wire(line: &str) -> Self {
        let mut parts = line.splitn(3, '\u{1f}');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(level), Some(at), Some(message)) => Self {
                level: LogLevel::parse(level).unwrap_or(LogLevel::Info),
                at_ms: at.parse().unwrap_or(0),
                message: message.to_owned(),
            },
            // An unpacked line came from somewhere else, and losing it would be
            // worse than showing it without a level.
            _ => Self {
                level: LogLevel::Info,
                at_ms: 0,
                message: line.to_owned(),
            },
        }
    }
}

/// One workspace, as the engine hands it to a configuration.
///
/// Mirrors `morf_wayland::WorkspaceInfo` rather than re-exporting it, so the
/// Lua runtime does not depend on the Wayland crate — the same separation the
/// window list beside it keeps.
#[derive(Clone, Debug, Default)]
pub struct Workspace {
    /// The field to act on, and what `morf.workspace.activate` takes.
    pub key: String,
    /// The compositor's cross-session id, when it offers one. Often empty --
    /// the protocol makes it optional and Hyprland sends none.
    pub id: String,
    /// What to show a person.
    pub name: String,
    /// Where it sits in the compositor's arrangement.
    pub coordinates: Vec<u32>,
    /// The output it belongs to, so a per-screen bar can filter to its own.
    pub output: String,
    pub active: bool,
    pub urgent: bool,
    pub hidden: bool,
    /// Whether `morf.workspace.activate` will do anything for it.
    pub activatable: bool,
}

/// One window the compositor reports, as handed to a configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Toplevel {
    /// Stable for the window's life, and the field to key on.
    pub identifier: String,
    /// What the window calls itself.
    pub title: String,
    /// Which application it belongs to.
    pub app_id: String,
    pub activated: bool,
    pub maximized: bool,
    pub minimized: bool,
    pub fullscreen: bool,
    /// Whether the window can be acted on. False means the state above is
    /// unknown rather than false -- the compositor offers no control protocol,
    /// or this window did not match a handle in it.
    pub controllable: bool,
}

/// Something a configuration asked to do to another window.
#[derive(Clone, Debug)]
pub struct ToplevelRequest {
    pub identifier: String,
    pub action: String,
    pub value: bool,
    /// For `set_minimize_target`: x, y, width, height on the shell's surface.
    pub rect: Option<(i32, i32, i32, i32)>,
}

/// Output metadata exposed to one per-screen Lua configuration instance.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Screen {
    pub id: u32,
    pub name: String,
    pub make: String,
    pub model: String,
    pub description: Option<String>,
    pub position: Option<(i32, i32)>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub physical_size: Option<(i32, i32)>,
    pub scale: i32,
    pub transform: String,
}

#[derive(Clone, Copy)]
pub(crate) enum StorageKind {
    Data,
    State,
    Cache,
}

pub(crate) fn shell_storage_dir(shell_root: &Path, kind: StorageKind) -> Result<PathBuf, String> {
    let (variable, fallback) = match kind {
        StorageKind::Data => ("XDG_DATA_HOME", ".local/share"),
        StorageKind::State => ("XDG_STATE_HOME", ".local/state"),
        StorageKind::Cache => ("XDG_CACHE_HOME", ".cache"),
    };
    let base = std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
        .ok_or_else(|| format!("{variable} and HOME are unset"))?;
    Ok(base.join("morf").join(shell_storage_key(shell_root)))
}

pub(crate) fn shell_storage_key(shell_root: &Path) -> String {
    let name = shell_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("shell")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let hash = shell_root
        .to_string_lossy()
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    format!("{name}-{hash:016x}")
}

pub(crate) fn launch_time_ms() -> u64 {
    static LAUNCH_TIME: OnceLock<u64> = OnceLock::new();
    *LAUNCH_TIME.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    })
}

pub(crate) fn rooted_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.len() > 4_096 || relative.as_bytes().contains(&0) {
        return Err("relative path is invalid".to_owned());
    }
    let relative = Path::new(relative);
    if relative.is_absolute() {
        return Err("relative path must not be absolute".to_owned());
    }
    Ok(root.join(relative))
}

pub(crate) fn icon_lookup_options(
    name: &str,
    theme: Option<String>,
    size: Option<i64>,
) -> Result<(String, u32), String> {
    if name.is_empty() || name.len() > 512 || name.as_bytes().contains(&0) {
        return Err("icon name is invalid".to_owned());
    }
    let theme = theme
        .or_else(|| std::env::var("MORF_ICON_THEME").ok())
        .unwrap_or_else(|| "hicolor".to_owned());
    if theme.is_empty() || theme.len() > 128 || theme.as_bytes().contains(&0) {
        return Err("icon theme is invalid".to_owned());
    }
    let size = u32::try_from(size.unwrap_or(32))
        .ok()
        .filter(|size| (1..=1_024).contains(size))
        .ok_or_else(|| "icon size must be 1..1024".to_owned())?;
    Ok((theme, size))
}

pub(crate) fn screen_density(screen: &Screen) -> Option<f64> {
    let (width, height) = (screen.width?, screen.height?);
    let (physical_width, physical_height) = screen.physical_size?;
    if width <= 0 || height <= 0 || physical_width <= 0 || physical_height <= 0 {
        return None;
    }
    let scale = f64::from(screen.scale.max(1));
    let horizontal = f64::from(width) * scale * 25.4 / f64::from(physical_width);
    let vertical = f64::from(height) * scale * 25.4 / f64::from(physical_height);
    Some((horizontal + vertical) / 2.0)
}

pub(crate) fn screen_primary_orientation(screen: &Screen) -> &'static str {
    let dimensions = screen
        .physical_size
        .or_else(|| screen.width.zip(screen.height));
    match dimensions {
        Some((width, height)) if width < height => "portrait",
        _ => "landscape",
    }
}

pub(crate) fn screen_orientation(screen: &Screen) -> &'static str {
    let primary = screen_primary_orientation(screen);
    match screen.transform.as_str() {
        "180" | "flipped_180" if primary == "portrait" => "inverted_portrait",
        "180" | "flipped_180" => "inverted_landscape",
        "90" | "flipped_90" if primary == "portrait" => "landscape",
        "90" | "flipped_90" => "portrait",
        "270" | "flipped_270" if primary == "portrait" => "inverted_landscape",
        "270" | "flipped_270" => "inverted_portrait",
        _ => primary,
    }
}
