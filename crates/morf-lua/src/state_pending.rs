//! Native work in flight, and who to tell when it finishes.
//!
//! Split from `state` at the line gate, and these belong together anyway: each
//! is a job the engine is running on a configuration's behalf -- a timer, an
//! authentication, a bus name, a subscription -- paired with the closure that
//! is owed the answer. The runtime drains all of them in one pass.

use luna::StashedClosure;
use morf_io::{DbusProxy, DbusService, DbusSignal, FileWatcher, Timer as IoTimer};
use morf_reactive::SignalId;
use morf_scene::NodeHandle;
use morf_services::{PamSession, PamTask, StatusNotifierHost, UdevMonitor};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct PendingPam {
    pub(crate) task: PamTask,
    pub(crate) callback: StashedClosure,
    pub(crate) unlock_on_success: bool,
}

pub(crate) struct PendingTimer {
    /// Names the timer to whoever holds its handle, so it can be cancelled.
    pub(crate) id: u64,
    pub(crate) timer: IoTimer,
    pub(crate) callback: StashedClosure,
    pub(crate) repeat: bool,
    pub(crate) interval: Duration,
    pub(crate) node: Option<NodeHandle>,
}

pub(crate) struct PendingDbusSignal {
    pub(crate) signal: DbusSignal,
    pub(crate) callback: StashedClosure,
}

/// A bus name this configuration owns, and who answers calls on it.
///
/// The service is shared rather than owned here because it is reachable from
/// two directions at once: the runtime polls it for arriving calls, and the
/// configuration replies through the same handle from inside the callback
/// those calls are delivered to.
pub(crate) struct PendingDbusService {
    pub(crate) service: Rc<RefCell<DbusService>>,
    pub(crate) callback: StashedClosure,
}

/// A PAM conversation in progress, and who is shown its messages.
///
/// Shared with the token for the same reason the D-Bus service is: the runtime
/// polls it for what the module said, and the configuration answers through
/// the same handle from inside the callback that showed it the question.
pub(crate) struct PendingPamSession {
    pub(crate) session: Rc<RefCell<PamSession>>,
    pub(crate) callback: StashedClosure,
}

pub(crate) struct PendingUdev {
    pub(crate) monitor: UdevMonitor,
    pub(crate) callback: StashedClosure,
}

pub(crate) struct PendingStatusNotifier {
    pub(crate) host: StatusNotifierHost,
    pub(crate) callback: StashedClosure,
}

/// A JSON file a theme takes its tokens from, watched for rewrites.
pub(crate) struct ThemeSource {
    pub(crate) path: PathBuf,
    /// Absent when the directory could not be watched; the file is then read
    /// once and never again.
    pub(crate) watcher: Option<FileWatcher>,
    /// The token each leaf key of the file writes.
    pub(crate) fields: HashMap<String, SignalId>,
}

/// The signals behind `morf.prefers`, and the settings portal they follow.
pub(crate) struct Prefers {
    pub(crate) color_scheme: SignalId,
    pub(crate) contrast: SignalId,
    pub(crate) reduced_motion: SignalId,
    pub(crate) accent_color: SignalId,
    pub(crate) scale: SignalId,
    /// The portal's settings interface and its change signal, when there is
    /// a portal to ask.
    pub(crate) portal: Option<(DbusProxy, DbusSignal)>,
}
