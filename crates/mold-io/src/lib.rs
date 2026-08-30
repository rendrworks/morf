//! Bounded process, file, socket, and timer primitives for mold.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::mem::MaybeUninit;
use std::net::Shutdown;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use zbus::blocking::{Connection as DbusConnection, Proxy as ZbusProxy};
use zbus::zvariant::{
    Array, Dict, DynamicDeserialize, DynamicType, ObjectPath, OwnedValue, Signature, Structure,
    StructureBuilder, Value,
};

include!("process.rs");
include!("streams.rs");
include!("files.rs");
include!("sockets.rs");
include!("ipc.rs");
include!("timer.rs");
include!("dbus_types.rs");
include!("dbus_encode.rs");
include!("dbus_decode.rs");
#[cfg(test)]
mod tests;
