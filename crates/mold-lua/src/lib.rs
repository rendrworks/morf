//! Sandboxed execution of mold configuration code.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use luna::{
    Callback, CallbackReturn, Closure, Context, Executor, ExecutorMode, Fuel, Function, Lua,
    StashedClosure, Table, UserData, UserRef, Value as LuaValue, Variadic,
};
use mold_desktop::{DesktopEntries, DesktopEntry, desktop_paths};
use mold_image::{IconResolver, ImageRect as QuantizeRect, quantize_colors};
use mold_io::{
    Bus, DbusProxy, DbusSignal, DbusValue, FileDocument, FileEvent, FileView, FileWatcher,
    LineParser, Process, ProcessConfig, ProcessEvent, Socket, SocketServer, SplitParser,
    StreamCollector, Timer as IoTimer,
};
use mold_layout::{Geometry, Layout, TransformTracker, TransformWatcher as NativeTransformWatcher};
use mold_lifecycle::Retention;
use mold_menu::{ButtonType, CheckState, Menu, MenuEntry};
use mold_reactive::{EffectContext, Graph, SignalId};
use mold_region::{Operation as RegionOperation, Rect as RegionRect, Region, Shape as RegionShape};
use mold_scene::{
    AnimationEnd, AnimationFrame, AnimationStep, Behavior, Color, Easing, Element, FlickState,
    GroupId, Keyframe, ListChange, ListModel, ModelId, NodeHandle, Physics, Repeat,
    RotationDirection, Scene, SceneError, Value as SceneValue, ViewTransition, VirtualList,
    keyframe_steps,
};
use mold_services::{
    AuthMessageType, GreetdClient, GreetdResponse, PamAuthenticator, PamTask, PipeWire,
    StatusNotifierAddress, StatusNotifierHost, UdevEvent, UdevMonitor, XkbKeymap,
};

include!("types.rs");
include!("surface_types.rs");
include!("events.rs");
include!("runtime_config.rs");
include!("runtime_screens.rs");
include!("runtime_events.rs");
include!("runtime_input.rs");
include!("runtime_animation.rs");
include!("runtime_services.rs");
include!("runtime_ipc.rs");
include!("runtime_helpers.rs");
include!("state.rs");
include!("scene_bindings.rs");
include!("api_signal.rs");
include!("api_retention.rs");
include!("api_shell.rs");
include!("api_time.rs");
include!("api_image.rs");
include!("api_transform.rs");
include!("api_animation.rs");
include!("api_group.rs");
include!("api_host.rs");
include!("api_view.rs");
include!("api_process.rs");
include!("api_file.rs");
include!("api_socket.rs");
include!("api_system.rs");
include!("api_ui_json.rs");
include!("api_menu.rs");
include!("api_module.rs");
include!("api_finish.rs");
include!("serialization.rs");
include!("process_helpers.rs");
include!("window_methods.rs");
include!("window_geometry.rs");
include!("constructors.rs");
include!("views.rs");
include!("configure.rs");
include!("table_menu.rs");
include!("window_parse.rs");
include!("layer_parse.rs");
include!("lua_values.rs");
include!("reactive_bindings.rs");
include!("reactive_execute.rs");
include!("runtime_default.rs");
#[cfg(test)]
mod tests;
