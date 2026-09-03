//! What a configuration holds in its hand.
//!
//! Every userdata a Lua configuration can be handed -- a signal, a process, a
//! socket, a bus name, a PAM conversation -- is one of these. Split from
//! `state` at the line gate; they belong together anyway, being the whole set
//! of things the engine lets a configuration keep a reference to.

use luna::{StashedClosure, UserRef};
use morf_desktop::DesktopEntries;
use morf_image::ImageRect as QuantizeRect;

use crate::state::ReactiveState;
use morf_io::{
    DbusProxy, DbusService, FileDocument, FileView, FileWatcher, Process, ProcessConfig, Socket,
    SocketServer, SplitParser, StreamCollector,
};
use morf_menu::Menu;
use morf_reactive::SignalId;
use morf_scene::{Easing, GroupId, ListModel, NodeHandle, VirtualList};
use morf_services::{GreetdClient, PamSession};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

#[derive(Debug)]
pub(crate) struct SignalToken {
    pub(crate) id: SignalId,
}

pub(crate) struct PersistentToken {
    pub(crate) properties: HashMap<String, SignalId>,
    pub(crate) reloaded: bool,
}

pub(crate) struct ScopeToken {
    pub(crate) prefix: String,
}

pub(crate) struct RetainableToken {
    pub(crate) node: NodeHandle,
}

pub(crate) struct WindowSurfaceToken {
    pub(crate) id: u64,
}

pub(crate) type PopupAnchorArgs<'gc> = (
    UserRef<'gc, WindowSurfaceToken>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

pub(crate) type WindowMapRectArgs<'gc> = (
    UserRef<'gc, WindowSurfaceToken>,
    UserRef<'gc, NodeToken>,
    f64,
    f64,
    f64,
    f64,
);

pub(crate) struct TransformWatcherToken {
    pub(crate) id: u64,
}

pub(crate) struct RetainLockToken {
    pub(crate) node: NodeHandle,
    pub(crate) locked: Cell<bool>,
    pub(crate) state: Rc<RefCell<ReactiveState>>,
}

impl Drop for RetainLockToken {
    fn drop(&mut self) {
        if !self.locked.get() {
            return;
        }
        if let Ok(mut state) = self.state.try_borrow_mut()
            && state.retention.unlock(self.node).is_ok()
            && state.retention.should_destroy(self.node).unwrap_or(false)
        {
            state.retained_destroy_queue.insert(self.node);
        }
    }
}

#[derive(Debug)]
pub(crate) struct NodeToken {
    pub(crate) handle: NodeHandle,
}

pub(crate) struct GroupToken {
    pub(crate) id: GroupId,
}

#[derive(Debug)]
pub(crate) struct DbusToken {
    pub(crate) proxy: DbusProxy,
}

pub(crate) struct DbusServiceToken {
    pub(crate) service: Rc<RefCell<DbusService>>,
}

pub(crate) struct PamSessionToken {
    pub(crate) session: Rc<RefCell<PamSession>>,
}

pub(crate) struct GreetdToken {
    pub(crate) client: RefCell<GreetdClient>,
}

pub(crate) struct ProcessToken {
    pub(crate) process: RefCell<Process>,
}

pub(crate) struct ProcessViewToken {
    pub(crate) state: RefCell<ProcessViewState>,
}

pub(crate) struct ProcessViewState {
    pub(crate) config: ProcessConfig,
    pub(crate) process: Option<Process>,
}

pub(crate) struct FileToken {
    pub(crate) file: FileView,
}

pub(crate) struct FileWatcherToken {
    pub(crate) watcher: FileWatcher,
}

pub(crate) struct FileDocumentToken {
    pub(crate) file: RefCell<FileDocument>,
}

pub(crate) struct SocketToken {
    pub(crate) state: RefCell<SocketState>,
}

pub(crate) struct SocketState {
    pub(crate) path: String,
    pub(crate) socket: Option<Socket>,
}

pub(crate) struct SocketServerToken {
    pub(crate) state: RefCell<SocketServerState>,
}

pub(crate) struct SocketServerState {
    pub(crate) path: String,
    pub(crate) server: Option<SocketServer>,
}

pub(crate) struct SplitParserToken {
    pub(crate) parser: RefCell<SplitParser>,
}

pub(crate) struct StreamCollectorToken {
    pub(crate) collector: RefCell<StreamCollector>,
}

pub(crate) struct ListModelToken {
    pub(crate) model: Rc<RefCell<ListModel>>,
}

pub(crate) struct VirtualListToken {
    pub(crate) model: Rc<RefCell<ListModel>>,
    pub(crate) view: RefCell<VirtualList>,
}

pub(crate) struct ElapsedTimerToken {
    pub(crate) started: RefCell<Instant>,
}

pub(crate) struct EasingCurveToken {
    pub(crate) easing: Easing,
}

pub(crate) struct ColorQuantizerToken {
    pub(crate) state: RefCell<ColorQuantizerState>,
}

#[derive(Clone)]
pub(crate) struct ColorQuantizerState {
    pub(crate) source: PathBuf,
    pub(crate) depth: u8,
    pub(crate) crop: Option<QuantizeRect>,
    pub(crate) rescale_size: u32,
    pub(crate) colors: Vec<[u8; 4]>,
}

pub(crate) struct SystemClockToken {
    pub(crate) enabled: Cell<bool>,
    pub(crate) precision: RefCell<String>,
}

pub(crate) struct JsonNullToken;

pub(crate) struct DesktopEntriesToken {
    pub(crate) entries: RefCell<DesktopEntries>,
    pub(crate) paths: Vec<PathBuf>,
}

pub(crate) struct MenuToken {
    pub(crate) menu: RefCell<Menu>,
    pub(crate) callbacks: HashMap<String, StashedClosure>,
}
