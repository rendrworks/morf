use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mold_io::{IpcIncoming, IpcReply, IpcRequest, IpcServer, IpcValue as WireValue, ipc_call};
use mold_layout::{Hit, Layout, ReparentTransition, Size};
use mold_lua::{
    EventPoint, FloatingSurfaceConfig, InputMethodRequest, IpcValue, LayerSurfaceConfig, Limits,
    PopupSurfaceConfig, Runtime, Screen, Screencopy as LuaScreencopy, TextInputRequest, UiEvent,
    VirtualKeyboardRequest, WindowSurfaceAction, WindowSurfaceConfig, WindowSurfaceKind,
};
use mold_render::{RenderEngine, WgpuBackend};
use mold_scene::{Element, NodeHandle};
use mold_wayland::{
    BarConfig, FloatingConfig, FloatingResizeEdge, InputRect, KeyboardFocus, LayerAnchors,
    LayerClient, LayerEvent, OutputPowerMode, PRIMARY_LAYER, PopupAnchor, PopupConfig,
    PopupConstraints, PopupGravity, ScreenInfo, ScreencopyFormat, ShellLayer, SurfaceRole,
};

include!("config.rs");
include!("lock.rs");
include!("supervisor.rs");
include!("workers.rs");
include!("services.rs");
include!("surfaces.rs");
include!("surface_layers.rs");
include!("surface_run.rs");
include!("surface_events.rs");
include!("surface_touch.rs");
include!("surface_actions.rs");
include!("paint.rs");

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mold: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
