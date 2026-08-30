fn supervise(path: PathBuf, source: Vec<u8>, policy: LoadPolicy) -> Result<(), String> {
    let probe = LayerClient::connect(BarConfig::default()).map_err(|error| error.to_string())?;
    let mut desired = named_screens(probe.screens())?;
    drop(probe);
    if desired.is_empty() {
        return Err("compositor advertised no named outputs".to_owned());
    }
    let path = Arc::new(path);
    let mut source: Arc<[u8]> = source.into();
    let (tx, rx) = mpsc::channel();
    let reload_roots = runtimepath_roots(&path, policy.external_roots);
    let mut reload_snapshot = lua_snapshot(&reload_roots);
    let watch_files = Arc::new(AtomicBool::new(true));
    let watcher_enabled = Arc::clone(&watch_files);
    let reload_tx = tx.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(100));
            let next = lua_snapshot(&reload_roots);
            if !watcher_enabled.load(Ordering::Acquire) {
                reload_snapshot = next;
                continue;
            }
            if next != reload_snapshot {
                reload_snapshot = next;
                if reload_tx
                    .send(SupervisorMessage::Reload { hard: false })
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    let (ipc_tx, ipc_rx) = mpsc::channel();
    let socket = socket_path()?;
    let server = IpcServer::bind(&socket, ipc_tx)
        .map_err(|error| format!("could not bind IPC socket {}: {error}", socket.display()))?;
    let owner = fs::metadata(&socket)
        .map_err(|error| format!("could not inspect IPC socket: {error}"))?
        .uid();
    let forward = tx.clone();
    thread::spawn(move || {
        while let Ok(request) = ipc_rx.recv() {
            if forward.send(SupervisorMessage::Ipc(request)).is_err() {
                break;
            }
        }
    });
    let mut workers = BTreeMap::new();
    let mut daemon_logs = Vec::new();
    reconcile_workers(
        &mut workers,
        &desired,
        Arc::clone(&path),
        Arc::clone(&source),
        policy,
        &tx,
    );

    loop {
        match rx.recv() {
            Ok(SupervisorMessage::Worker(WorkerMessage::Screens { output, screens }))
                if workers.contains_key(&output) =>
            {
                desired = named_screens(&screens)?;
                reconcile_workers(
                    &mut workers,
                    &desired,
                    Arc::clone(&path),
                    Arc::clone(&source),
                    policy,
                    &tx,
                );
            }
            Ok(SupervisorMessage::Worker(WorkerMessage::Screens { .. })) => {}
            Ok(SupervisorMessage::Worker(WorkerMessage::Failed { output, error })) => {
                stop_workers(workers);
                return Err(format!("output {output}: {error}"));
            }
            Ok(SupervisorMessage::Ipc(incoming)) => {
                if incoming.peer.uid != owner {
                    incoming.reply(IpcReply::refused("peer uid does not own the shell"));
                    continue;
                }
                let kill = matches!(incoming.request, IpcRequest::Kill);
                let reply = handle_ipc(&workers, &mut daemon_logs, &incoming.request);
                incoming.reply(reply);
                if kill {
                    stop_workers(workers);
                    drop(server);
                    return Ok(());
                }
            }
            Ok(SupervisorMessage::Reload { hard }) => match fs::read(path.as_ref()) {
                Ok(bytes) => {
                    source = Arc::from(bytes);
                    for (output, worker) in &workers {
                        let (reply, result) = mpsc::sync_channel(1);
                        if worker
                            .commands
                            .send(WorkerCommand::Reload {
                                path: Arc::clone(&path),
                                source: Arc::clone(&source),
                                hard,
                                reply,
                            })
                            .is_err()
                        {
                            daemon_logs.push(format!("reload {output}: output stopped"));
                            continue;
                        }
                        match result.recv_timeout(Duration::from_secs(2)) {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => daemon_logs.push(format!("reload {output}: {error}")),
                            Err(_) => daemon_logs.push(format!("reload {output}: timed out")),
                        }
                    }
                }
                Err(error) => daemon_logs.push(format!("reload: {error}")),
            },
            Ok(SupervisorMessage::WatchFiles(enabled)) => {
                watch_files.store(enabled, Ordering::Release);
            }
            Err(_) => return Err("all output workers stopped".to_owned()),
        }
    }
}

fn named_screens(screens: &[ScreenInfo]) -> Result<BTreeMap<String, ScreenInfo>, String> {
    screens
        .iter()
        .map(|screen| {
            screen
                .name
                .clone()
                .map(|name| (name, screen.clone()))
                .ok_or_else(|| format!("output {} has no compositor name", screen.id))
        })
        .collect()
}

fn runtimepath_roots(config: &Path, external: bool) -> Vec<PathBuf> {
    let mut roots = config
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect::<Vec<_>>();
    if external {
        roots.extend(
            env::var_os("MOLD_RUNTIME_PATH")
                .into_iter()
                .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>()),
        );
        if let Some(data) = env::var_os("XDG_DATA_HOME") {
            roots.push(PathBuf::from(data).join("mold/site"));
        } else if let Some(home) = env::var_os("HOME") {
            roots.push(PathBuf::from(home).join(".local/share/mold/site"));
        }
    }
    let mut unique = Vec::new();
    for root in roots {
        if !unique.contains(&root) {
            unique.push(root);
        }
    }
    unique
}

fn execute_config(
    runtime: &mut Runtime,
    path: &Path,
    source: &[u8],
    policy: LoadPolicy,
) -> Result<(), String> {
    let roots = runtimepath_roots(path, policy.external_roots);
    runtime.set_module_roots(roots.clone());
    runtime.set_shell_root(
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    );
    for plugin in policy
        .plugins
        .then(|| runtime_scripts(&roots, "plugin"))
        .into_iter()
        .flatten()
    {
        match fs::read(&plugin) {
            Ok(source) => {
                if let Err(error) = runtime.execute(&plugin.to_string_lossy(), &source) {
                    eprintln!("mold: plugin {}: {error}", plugin.display());
                }
            }
            Err(error) => eprintln!("mold: plugin {}: {error}", plugin.display()),
        }
    }
    runtime
        .execute(&path.to_string_lossy(), source)
        .map_err(|error| error.to_string())?;
    for after in policy
        .plugins
        .then(|| runtime_scripts(&roots, "after/plugin"))
        .into_iter()
        .flatten()
    {
        match fs::read(&after) {
            Ok(source) => {
                if let Err(error) = runtime.execute(&after.to_string_lossy(), &source) {
                    eprintln!("mold: after plugin {}: {error}", after.display());
                }
            }
            Err(error) => eprintln!("mold: after plugin {}: {error}", after.display()),
        }
    }
    Ok(())
}

fn runtime_scripts(roots: &[PathBuf], directory: &str) -> Vec<PathBuf> {
    let mut scripts = Vec::new();
    for root in roots {
        let mut found = Vec::new();
        collect_lua_scripts(&root.join(directory), &mut found);
        found.sort();
        for path in found {
            if !scripts.contains(&path) {
                scripts.push(path);
            }
        }
    }
    scripts
}

fn collect_lua_scripts(path: &Path, scripts: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lua_scripts(&path, scripts);
        } else if path.extension().and_then(|value| value.to_str()) == Some("lua") {
            scripts.push(path);
        }
    }
}

fn lua_snapshot(roots: &[PathBuf]) -> BTreeMap<PathBuf, (u64, SystemTime)> {
    let mut snapshot = BTreeMap::new();
    let mut pending = roots.to_vec();
    while let Some(path) = pending.pop() {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("lua") {
                continue;
            }
            if let Ok(metadata) = entry.metadata() {
                snapshot.insert(
                    path,
                    (
                        metadata.len(),
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    ),
                );
            }
        }
    }
    snapshot
}

