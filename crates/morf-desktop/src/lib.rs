//! XDG desktop entry parsing, discovery, lookup, and launching.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MAX_ENTRY_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 16_384;
const MAX_DEPTH: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub exec: String,
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopEntry {
    pub id: String,
    pub name: String,
    pub generic_name: String,
    pub startup_class: String,
    pub no_display: bool,
    pub hidden: bool,
    /// A program that must exist for the entry to be offered at all.
    pub try_exec: String,
    /// What the session calls the desktop it starts, for `XDG_CURRENT_DESKTOP`.
    pub desktop_names: Vec<String>,
    /// The directory the entry was read from, which is how a session says
    /// whether it is Wayland or X11 — the only place that is written down.
    pub source: String,
    pub comment: String,
    pub icon: String,
    pub exec: String,
    pub command: Vec<String>,
    pub working_directory: String,
    pub run_in_terminal: bool,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub actions: Vec<DesktopAction>,
}

impl DesktopEntry {
    pub fn parse(id: impl Into<String>, source: &str) -> Option<Self> {
        let id = id.into();
        let mut groups = BTreeMap::<String, BTreeMap<String, String>>::new();
        let mut group = String::new();
        for raw in source.lines() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
            {
                group = name.to_owned();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.contains('[') {
                continue;
            }
            groups
                .entry(group.clone())
                .or_default()
                .insert(key.to_owned(), value.to_owned());
        }
        let main = groups.get("Desktop Entry")?;
        if main.get("Type").map(String::as_str) != Some("Application") {
            return None;
        }
        let name = main.get("Name")?.clone();
        let action_order = list_value(main.get("Actions"));
        let actions = action_order
            .into_iter()
            .filter_map(|id| {
                let values = groups.get(&format!("Desktop Action {id}"))?;
                let exec = values.get("Exec").cloned().unwrap_or_default();
                Some(DesktopAction {
                    id,
                    name: values.get("Name").cloned().unwrap_or_default(),
                    icon: values.get("Icon").cloned().unwrap_or_default(),
                    command: parse_exec(&exec),
                    exec,
                })
            })
            .collect();
        let exec = main.get("Exec").cloned().unwrap_or_default();
        Some(Self {
            id,
            name,
            generic_name: value(main, "GenericName"),
            startup_class: value(main, "StartupWMClass"),
            no_display: boolean(main, "NoDisplay"),
            hidden: boolean(main, "Hidden"),
            try_exec: value(main, "TryExec"),
            desktop_names: list_value(main.get("DesktopNames")),
            source: String::new(),
            comment: value(main, "Comment"),
            icon: value(main, "Icon"),
            command: parse_exec(&exec),
            exec,
            working_directory: value(main, "Path"),
            run_in_terminal: boolean(main, "Terminal"),
            categories: list_value(main.get("Categories")),
            keywords: list_value(main.get("Keywords")),
            actions,
        })
    }

    pub fn launch(&self) -> io::Result<()> {
        launch(&self.command, &self.working_directory)
    }
}

impl DesktopAction {
    pub fn launch(&self, working_directory: &str) -> io::Result<()> {
        launch(&self.command, working_directory)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesktopEntries {
    entries: Vec<DesktopEntry>,
}

impl DesktopEntries {
    pub fn scan_paths(paths: impl IntoIterator<Item = PathBuf>) -> io::Result<Self> {
        let mut entries = Vec::new();
        let mut claimed = HashSet::new();
        for path in paths {
            scan_directory(&path, &path, 0, &mut entries, &mut claimed)?;
            if claimed.len() >= MAX_ENTRIES {
                break;
            }
        }
        entries.sort_by_key(|entry| entry.name.to_lowercase());
        Ok(Self { entries })
    }

    pub fn applications(&self) -> &[DesktopEntry] {
        &self.entries
    }

    pub fn by_id(&self, id: &str) -> Option<&DesktopEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn heuristic_lookup(&self, query: &str) -> Option<&DesktopEntry> {
        let query = query.to_lowercase();
        self.entries
            .iter()
            .find(|entry| entry.id.to_lowercase() == query)
            .or_else(|| {
                self.entries.iter().find(|entry| {
                    entry.name.to_lowercase() == query
                        || entry.startup_class.to_lowercase() == query
                })
            })
    }
}

pub fn desktop_paths() -> Vec<PathBuf> {
    data_paths(&["applications"])
}

/// Where the sessions a greeter can start are described.
///
/// The same entry format and the same search order as applications, in two
/// other directories. A login screen's whole job is to list these and run the
/// one that is picked, and until now the only directory this crate would look
/// in was the one sessions are *not* in.
///
/// Wayland first, deliberately: where a desktop ships both, they name the same
/// session and the Wayland one is the one to prefer.
pub fn session_paths() -> Vec<PathBuf> {
    const KINDS: [&str; 2] = ["wayland-sessions", "xsessions"];
    let mut paths = data_paths(&KINDS);
    // And the canonical system directories, whatever `XDG_DATA_DIRS` says.
    //
    // Not pedantry about the spec — the spec is honoured above. This is that a
    // greeter has to work in whatever environment it is handed, and that
    // environment is not the one that installed the sessions. `greetd` runs the
    // greeter as its own user; a development shell may replace `XDG_DATA_DIRS`
    // wholesale, which is exactly what the shell this was written in does, so
    // that a machine with `/usr/share/wayland-sessions/hyprland.desktop` right
    // there listed no sessions at all.
    //
    // Every display manager looks here for the same reason. Deduplicated, so
    // naming it twice does not scan it twice.
    for kind in KINDS {
        let canonical = PathBuf::from("/usr/share").join(kind);
        if !paths.contains(&canonical) {
            paths.push(canonical);
        }
    }
    paths
}

/// Every XDG data directory, suffixed by each of `kinds`, in search order.
///
/// The user's own first, then the system's, which is the order that lets
/// somebody override a system entry by shadowing its file name.
fn data_paths(kinds: &[&str]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    roots.extend(data_home);
    let data_dirs =
        std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    roots.extend(std::env::split_paths(&data_dirs));
    roots
        .into_iter()
        .flat_map(|root| kinds.iter().map(move |kind| root.join(kind)))
        .collect()
}

pub fn parse_exec(source: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = None;
    let mut escaped = false;
    let mut chars = source.chars().peekable();
    while let Some(character) = chars.next() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if quoted == Some(character) {
            quoted = None;
        } else if quoted.is_none() && matches!(character, '\'' | '"') {
            quoted = Some(character);
        } else if character == '%' {
            if chars.peek() == Some(&'%') {
                chars.next();
                current.push('%');
            } else {
                chars.next();
            }
        } else if quoted.is_none() && character.is_whitespace() {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

/// Whether `TryExec` names something runnable. An empty key claims nothing.
fn program_exists(try_exec: &str) -> bool {
    let name = try_exec.trim();
    if name.is_empty() {
        return true;
    }
    if name.contains('/') {
        return fs::metadata(name).is_ok_and(|found| found.is_file());
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|directory| fs::metadata(directory.join(name)).is_ok_and(|found| found.is_file()))
}

fn value(values: &BTreeMap<String, String>, key: &str) -> String {
    values.get(key).cloned().unwrap_or_default()
}

fn boolean(values: &BTreeMap<String, String>, key: &str) -> bool {
    values.get(key).is_some_and(|value| value == "true")
}

fn list_value(value: Option<&String>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|value| value.split(';'))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn launch(command: &[String], working_directory: &str) -> io::Result<()> {
    let (program, args) = command.split_first().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "desktop entry command is empty",
        )
    })?;
    let mut process = Command::new(program);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !working_directory.is_empty() {
        process.current_dir(working_directory);
    }
    process.spawn().map(|_| ())
}

fn scan_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    entries: &mut Vec<DesktopEntry>,
    claimed: &mut HashSet<String>,
) -> io::Result<()> {
    if depth > MAX_DEPTH || claimed.len() >= MAX_ENTRIES || !directory.exists() {
        return Ok(());
    }
    let mut children = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        if path.is_dir() {
            scan_directory(root, &path, depth + 1, entries, claimed)?;
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("desktop") {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let id = relative
            .with_extension("")
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("-");
        if !claimed.insert(id.clone()) || claimed.len() > MAX_ENTRIES {
            continue;
        }
        if fs::metadata(&path)?.len() > MAX_ENTRY_BYTES {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(mut entry) = DesktopEntry::parse(id, &source)
            && !entry.hidden
            && !entry.no_display
            // An entry naming a program that is not installed is an entry that
            // cannot run. Left in, it is a session in the greeter's list that
            // authenticates and then dies with nothing on screen to say why.
            && program_exists(&entry.try_exec)
        {
            entry.source = directory.to_string_lossy().into_owned();
            entries.push(entry);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
