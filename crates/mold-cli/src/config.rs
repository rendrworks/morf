fn usage() -> &'static str {
    "mold - reactive Wayland shell runtime\n\nusage: mold [--no-plugin | --clean] [shell.lua]\n       mold [--no-plugin | --clean] -c <name>\n       mold lock [lock.lua]\n       mold ipc call <target> [args...]\n       mold ipc verbs\n       mold log [--bindings]\n       mold kill\n       mold --help\n       mold --version"
}

fn run() -> Result<(), String> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    match parse_command(&args)? {
        Command::Help => println!("{}", usage()),
        Command::Version => println!("mold {}", env!("CARGO_PKG_VERSION")),
        Command::Run(path, policy) => {
            let source = fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            supervise(path, source, policy)?;
        }
        Command::Lock(path) => {
            let source = fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            run_lock(&path, &source)?;
        }
        Command::Client(request) => {
            let reply = ipc_call(socket_path()?, &request).map_err(|error| error.to_string())?;
            println!(
                "{}",
                String::from_utf8_lossy(&reply.to_wire().map_err(|e| e.to_string())?)
            );
        }
    }
    Ok(())
}

enum Command {
    Help,
    Version,
    Run(PathBuf, LoadPolicy),
    Lock(PathBuf),
    Client(IpcRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LoadPolicy {
    plugins: bool,
    external_roots: bool,
}

impl Default for LoadPolicy {
    fn default() -> Self {
        Self {
            plugins: true,
            external_roots: true,
        }
    }
}

fn parse_command(args: &[std::ffi::OsString]) -> Result<Command, String> {
    let strings = args
        .iter()
        .map(|value| {
            value
                .to_str()
                .ok_or_else(|| "arguments must be UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (policy, strings) = match strings.as_slice() {
        ["--no-plugin", rest @ ..] => (
            LoadPolicy {
                plugins: false,
                external_roots: true,
            },
            rest,
        ),
        ["--clean", rest @ ..] => (
            LoadPolicy {
                plugins: false,
                external_roots: false,
            },
            rest,
        ),
        strings => (LoadPolicy::default(), strings),
    };
    match strings {
        ["-h" | "--help"] => Ok(Command::Help),
        ["-V" | "--version"] => Ok(Command::Version),
        ["-c", name] => Ok(Command::Run(named_config_path(name)?, policy)),
        ["lock"] => Ok(Command::Lock(config_root()?.join("lock.lua"))),
        ["lock", path] => Ok(Command::Lock(PathBuf::from(path))),
        ["ipc", "verbs"] => Ok(Command::Client(IpcRequest::Verbs)),
        ["ipc", "call", target, args @ ..] => Ok(Command::Client(IpcRequest::Call {
            target: (*target).to_owned(),
            args: args
                .iter()
                .map(|value| WireValue::String((*value).to_owned()))
                .collect(),
        })),
        ["log"] => Ok(Command::Client(IpcRequest::Log)),
        ["log", "--bindings"] => Ok(Command::Client(IpcRequest::Bindings)),
        ["kill"] => Ok(Command::Client(IpcRequest::Kill)),
        [] => Ok(Command::Run(config_root()?.join("shell.lua"), policy)),
        [path] => Ok(Command::Run(PathBuf::from(path), policy)),
        _ => Err(usage().to_owned()),
    }
}

fn config_root() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("mold"));
    }
    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config/mold"))
        .ok_or_else(|| "HOME and XDG_CONFIG_HOME are unset".to_owned())
}

fn named_config_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err("config name must be one path component".to_owned());
    }
    Ok(config_root()?.join(name).join("shell.lua"))
}

fn socket_path() -> Result<PathBuf, String> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is unset".to_owned())?;
    let display = env::var("WAYLAND_DISPLAY").map_err(|_| "WAYLAND_DISPLAY is unset".to_owned())?;
    if display.is_empty() || display.contains('/') {
        return Err("WAYLAND_DISPLAY must be one path component".to_owned());
    }
    Ok(runtime.join("mold").join(format!("{display}.sock")))
}

