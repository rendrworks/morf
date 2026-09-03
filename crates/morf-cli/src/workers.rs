use morf_io::{IpcReply, IpcRequest, IpcValue as WireValue};
use morf_lua::{Limits, LogEntry, Runtime, Screen};
use morf_wayland::ScreenInfo;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self};
use std::time::Duration;

use crate::{
    config::*, lock::*, paint::*, services::*, supervisor::*, surface_run::*, surfaces::*,
};

pub(crate) fn reconcile_workers(
    workers: &mut BTreeMap<String, Worker>,
    desired: &BTreeMap<String, ScreenInfo>,
    path: Arc<PathBuf>,
    source: Arc<[u8]>,
    policy: LoadPolicy,
    tx: &mpsc::Sender<SupervisorMessage>,
) {
    let stale = workers
        .iter()
        .filter(|(name, worker)| desired.get(*name) != Some(&worker.screen))
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    for name in stale {
        let worker = workers.remove(&name).expect("worker key is present");
        worker.stop.store(true, Ordering::Release);
        let _ = worker.join.join();
    }
    for (name, screen) in desired {
        if workers.contains_key(name) {
            continue;
        }
        let output = name.clone();
        let screen = screen.clone();
        let worker_screen = screen.clone();
        let path = Arc::clone(&path);
        let source = Arc::clone(&source);
        let tx = tx.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (commands, command_rx) = mpsc::channel();
        // Named after the output it drives, so anything reporting per-thread —
        // a profiler, a panic, a frame counter — says which screen it means.
        let join = thread::Builder::new()
            .name(output.clone())
            .spawn(move || {
                if let Err(error) = run_surface(
                    &path,
                    &source,
                    screen,
                    policy,
                    &tx,
                    &worker_stop,
                    &command_rx,
                ) && !worker_stop.load(Ordering::Acquire)
                {
                    let _ = tx.send(SupervisorMessage::Worker(WorkerMessage::Failed {
                        output,
                        error,
                    }));
                }
            })
            .expect("worker thread");
        workers.insert(
            name.clone(),
            Worker {
                stop,
                commands,
                join,
                screen: worker_screen,
            },
        );
    }
}

/// Hands every live worker the compositor's new output list, so each runtime's
/// `morf.screens` follows a monitor being plugged in, moved, or unplugged.
pub(crate) fn broadcast_screens(workers: &BTreeMap<String, Worker>, screens: &[ScreenInfo]) {
    for worker in workers.values() {
        let _ = worker
            .commands
            .send(WorkerCommand::Screens(screens.to_vec()));
    }
}

pub(crate) fn handle_ipc(
    workers: &BTreeMap<String, Worker>,
    daemon_logs: &mut Vec<String>,
    request: &IpcRequest,
) -> IpcReply {
    match request {
        IpcRequest::Call { target, args } => {
            let Some(worker) = workers.values().next() else {
                return IpcReply::refused("shell has no active output");
            };
            let args = args.iter().map(lua_ipc_value).collect::<Vec<_>>();
            let (tx, rx) = mpsc::sync_channel(1);
            if worker
                .commands
                .send(WorkerCommand::Call {
                    target: target.clone(),
                    args,
                    reply: tx,
                })
                .is_err()
            {
                return IpcReply::refused("shell output stopped");
            }
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(values)) => IpcReply::success(values.iter().map(wire_ipc_value).collect()),
                Ok(Err(error)) => IpcReply::refused(error),
                Err(_) => IpcReply::refused("shell output timed out"),
            }
        }
        IpcRequest::Verbs => {
            let mut verbs = Vec::new();
            for worker in workers.values() {
                let (tx, rx) = mpsc::sync_channel(1);
                if worker.commands.send(WorkerCommand::Verbs(tx)).is_ok()
                    && let Ok(found) = rx.recv_timeout(Duration::from_secs(1))
                {
                    verbs.extend(found);
                }
            }
            verbs.sort();
            verbs.dedup();
            IpcReply::success(verbs.into_iter().map(WireValue::String).collect())
        }
        IpcRequest::Log => {
            let mut logs = std::mem::take(daemon_logs);
            for worker in workers.values() {
                let (tx, rx) = mpsc::sync_channel(1);
                if worker.commands.send(WorkerCommand::Logs(tx)).is_ok()
                    && let Ok(found) = rx.recv_timeout(Duration::from_secs(1))
                {
                    logs.extend(found);
                }
            }
            IpcReply::success(logs.into_iter().map(WireValue::String).collect())
        }
        IpcRequest::Capabilities => {
            let mut lines = Vec::new();
            for (output, worker) in workers {
                let (tx, rx) = mpsc::sync_channel(1);
                if worker
                    .commands
                    .send(WorkerCommand::Capabilities(tx))
                    .is_ok()
                    && let Ok(found) = rx.recv_timeout(Duration::from_secs(1))
                {
                    lines.extend(found.into_iter().map(|line| format!("{output}:{line}")));
                }
            }
            IpcReply::success(lines.into_iter().map(WireValue::String).collect())
        }
        IpcRequest::Bindings => {
            let mut bindings = Vec::new();
            for worker in workers.values() {
                let (tx, rx) = mpsc::sync_channel(1);
                if worker.commands.send(WorkerCommand::Bindings(tx)).is_ok()
                    && let Ok(found) = rx.recv_timeout(Duration::from_secs(1))
                {
                    bindings.extend(found);
                }
            }
            bindings.sort();
            bindings.dedup();
            IpcReply::success(bindings.into_iter().map(WireValue::String).collect())
        }
        IpcRequest::Kill => IpcReply::success(Vec::new()),
        // The supervisor answers this before it reaches here; it is the one
        // thing that knows which configuration it is running.
        IpcRequest::Info => IpcReply::refused("info is answered by the supervisor"),
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct WorkerUpdate {
    pub(crate) repaint: bool,
    pub(crate) reset_input: bool,
    pub(crate) refresh_idle: bool,
    pub(crate) recreate_surface: bool,
}

pub(crate) fn handle_worker_command(
    runtime: &mut Runtime,
    screen: &Screen,
    policy: LoadPolicy,
    command: WorkerCommand,
) -> WorkerUpdate {
    match command {
        WorkerCommand::Call {
            target,
            args,
            reply,
        } => {
            let result = runtime
                .call_ipc(&target, &args)
                .map_err(|error| error.to_string());
            let repaint = result.is_ok();
            let _ = reply.send(result);
            WorkerUpdate {
                repaint,
                reset_input: false,
                refresh_idle: false,
                recreate_surface: false,
            }
        }
        WorkerCommand::Screens(screens) => {
            runtime.set_screens(&lua_screens(&screens));
            WorkerUpdate::default()
        }
        WorkerCommand::Verbs(reply) => {
            let _ = reply.send(runtime.ipc_verbs());
            WorkerUpdate::default()
        }
        WorkerCommand::Logs(reply) => {
            // Packed here rather than at the socket, because the entry is
            // structured on this side and a string by the time it leaves.
            let _ = reply.send(
                runtime
                    .take_logs()
                    .iter()
                    .map(LogEntry::to_wire)
                    .collect::<Vec<_>>(),
            );
            WorkerUpdate::default()
        }
        WorkerCommand::Capabilities(reply) => {
            let _ = reply.send(runtime.capabilities());
            WorkerUpdate::default()
        }
        WorkerCommand::Bindings(reply) => {
            let _ = reply.send(runtime.binding_dependencies());
            WorkerUpdate::default()
        }
        WorkerCommand::Reload {
            path,
            source,
            hard,
            reply,
        } => {
            let mut candidate = Runtime::for_screen(Limits::default(), screen.clone());
            if !hard {
                candidate.restore_reloadable_state(runtime.reloadable_state());
            }
            let result = execute_config(&mut candidate, &path, &source, policy)
                .and_then(|()| primary_surface_root(&candidate).map(|_| ()))
                .and_then(|()| {
                    candidate
                        .update_clock(clock_text())
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                });
            let repaint = match &result {
                Ok(()) => {
                    candidate.dispatch_reload_completed();
                    *runtime = candidate;
                    true
                }
                Err(error) => {
                    runtime.dispatch_reload_failed(error.clone());
                    false
                }
            };
            let _ = reply.send(result);
            WorkerUpdate {
                repaint,
                reset_input: repaint,
                refresh_idle: repaint,
                recreate_surface: repaint && hard,
            }
        }
    }
}
