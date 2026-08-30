//! Bounded process, file, socket, and timer primitives for mold.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
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

/// One event emitted by a child process.
#[derive(Debug)]
pub enum ProcessEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(ExitStatus),
}

/// Spawned child with streamed output and writable stdin.
pub struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    events: mpsc::Receiver<ProcessEvent>,
    exit_reported: bool,
}

/// Native process launch settings without shell interpretation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessConfig {
    pub command: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub clear_environment: bool,
    pub working_directory: Option<PathBuf>,
}

impl Process {
    /// Spawns a child without invoking a shell.
    pub fn spawn<I, S>(program: impl AsRef<std::ffi::OsStr>, args: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(program);
        command.args(args);
        Self::spawn_command(&mut command)
    }

    pub fn spawn_config(config: &ProcessConfig) -> io::Result<Self> {
        let (program, args) = config.command.split_first().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "process command cannot be empty",
            )
        })?;
        let mut command = Command::new(program);
        command.args(args);
        if config.clear_environment {
            command.env_clear();
        }
        command.envs(&config.environment);
        if let Some(directory) = &config.working_directory {
            command.current_dir(directory);
        }
        Self::spawn_command(&mut command)
    }

    fn spawn_command(command: &mut Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout is present");
        let stderr = child.stderr.take().expect("piped stderr is present");
        let (tx, events) = mpsc::channel();
        stream_reader(stdout, tx.clone(), ProcessEvent::Stdout);
        stream_reader(stderr, tx, ProcessEvent::Stderr);
        Ok(Self {
            child,
            stdin,
            events,
            exit_reported: false,
        })
    }

    /// Writes bytes to the child's standard input.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "child stdin is closed"))?
            .write_all(bytes)
    }

    /// Closes the child's standard input.
    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn signal(&self, signal: i32) -> io::Result<()> {
        if !(1..=64).contains(&signal) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "signal must be 1..64",
            ));
        }
        let result = unsafe { libc::kill(self.child.id() as i32, signal) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Waits up to the supplied duration for output or process exit.
    pub fn next_event(&mut self, timeout: Duration) -> io::Result<Option<ProcessEvent>> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => return Ok(Some(event)),
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }
        if !self.exit_reported
            && let Some(status) = self.child.try_wait()?
        {
            self.exit_reported = true;
            return Ok(Some(ProcessEvent::Exit(status)));
        }
        Ok(None)
    }

    /// Requests child termination.
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stdin = None;
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn stream_reader<R, F>(mut reader: R, tx: mpsc::Sender<ProcessEvent>, event: F)
where
    R: Read + Send + 'static,
    F: Fn(Vec<u8>) -> ProcessEvent + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = vec![0; 8 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) if tx.send(event(buffer[..read].to_vec())).is_err() => break,
                Ok(_) => {}
            }
        }
    });
}

/// Incremental newline-delimited byte parser.
#[derive(Default)]
pub struct LineParser {
    pending: Vec<u8>,
}

impl LineParser {
    /// Appends a chunk and returns every complete line without its delimiter.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(at) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=at).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            lines.push(String::from_utf8_lossy(&line).into_owned());
        }
        lines
    }

    /// Returns the final unterminated line.
    pub fn finish(&mut self) -> Option<String> {
        (!self.pending.is_empty())
            .then(|| String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned())
    }
}

/// Incremental byte parser using an arbitrary non-empty delimiter.
pub struct SplitParser {
    delimiter: Vec<u8>,
    pending: Vec<u8>,
}

impl SplitParser {
    /// Creates a parser for the supplied delimiter.
    pub fn new(delimiter: impl Into<Vec<u8>>) -> io::Result<Self> {
        let delimiter = delimiter.into();
        if delimiter.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "split delimiter cannot be empty",
            ));
        }
        Ok(Self {
            delimiter,
            pending: Vec::new(),
        })
    }

    /// Appends a chunk and returns every complete segment.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.pending.extend_from_slice(chunk);
        let mut parts = Vec::new();
        while let Some(at) = find_bytes(&self.pending, &self.delimiter) {
            parts.push(self.pending.drain(..at).collect());
            self.pending.drain(..self.delimiter.len());
        }
        parts
    }

    /// Returns the final unterminated segment.
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        (!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending))
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Bounded stream collector with optional end-of-stream publication.
pub struct StreamCollector {
    pending: Vec<u8>,
    data: Vec<u8>,
    maximum: usize,
    wait_for_end: bool,
    finished: bool,
}

impl StreamCollector {
    /// Creates a collector with an explicit byte limit.
    pub fn new(maximum: usize, wait_for_end: bool) -> io::Result<Self> {
        if maximum == 0 || maximum > 16 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream collector limit must be 1..16777216 bytes",
            ));
        }
        Ok(Self {
            pending: Vec::new(),
            data: Vec::new(),
            maximum,
            wait_for_end,
            finished: false,
        })
    }

    /// Appends bytes and returns whether the published value changed.
    pub fn push(&mut self, chunk: &[u8]) -> io::Result<bool> {
        if self.finished {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "stream collector is finished",
            ));
        }
        if self.pending.len().saturating_add(chunk.len()) > self.maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stream collector exceeded its byte limit",
            ));
        }
        self.pending.extend_from_slice(chunk);
        if self.wait_for_end {
            Ok(false)
        } else {
            self.data.clone_from(&self.pending);
            Ok(true)
        }
    }

    /// Publishes the final buffer and marks the stream finished.
    pub fn finish(&mut self) -> bool {
        if self.finished {
            return false;
        }
        self.finished = true;
        if self.wait_for_end {
            self.data.clone_from(&self.pending);
        }
        true
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.data).into_owned()
    }

    pub fn wait_for_end(&self) -> bool {
        self.wait_for_end
    }

    pub fn set_wait_for_end(&mut self, wait_for_end: bool) {
        if self.wait_for_end == wait_for_end {
            return;
        }
        self.wait_for_end = wait_for_end;
        if !wait_for_end {
            self.data.clone_from(&self.pending);
        }
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.data.clear();
        self.finished = false;
    }
}

/// Readable and atomically writable filesystem path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileView {
    path: PathBuf,
}

impl FileView {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn read(&self) -> io::Result<Vec<u8>> {
        fs::read(&self.path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Reads a file only when its current size fits the supplied bound.
    pub fn read_bounded(&self, maximum: usize) -> io::Result<Vec<u8>> {
        let length = fs::metadata(&self.path)?.len();
        if length > maximum as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file exceeds read limit",
            ));
        }
        self.read()
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.write_with_mode(bytes, true)
    }

    pub fn write_with_mode(&self, bytes: &[u8], atomic: bool) -> io::Result<()> {
        if !atomic {
            return fs::write(&self.path, bytes);
        }
        let mut temporary = self.path.clone();
        let extension = self
            .path
            .extension()
            .and_then(|value| value.to_str())
            .map_or_else(
                || "mold-tmp".to_owned(),
                |value| format!("{value}.mold-tmp"),
            );
        temporary.set_extension(extension);
        fs::write(&temporary, bytes)?;
        fs::rename(temporary, &self.path)
    }

    pub fn watch(&self) -> io::Result<FileWatcher> {
        FileWatcher::new(&self.path)
    }
}

/// Stable error category for a file load or save operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileViewError {
    FileNotFound,
    NotAFile,
    PermissionDenied,
    TooLarge,
    Unknown,
}

impl FileViewError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileNotFound => "file_not_found",
            Self::NotAFile => "not_a_file",
            Self::PermissionDenied => "permission_denied",
            Self::TooLarge => "too_large",
            Self::Unknown => "unknown",
        }
    }
}

fn classify_file_error(error: &io::Error) -> FileViewError {
    match error.kind() {
        io::ErrorKind::NotFound => FileViewError::FileNotFound,
        io::ErrorKind::PermissionDenied => FileViewError::PermissionDenied,
        io::ErrorKind::InvalidData if error.to_string() == "file exceeds read limit" => {
            FileViewError::TooLarge
        }
        io::ErrorKind::IsADirectory => FileViewError::NotAFile,
        _ => FileViewError::Unknown,
    }
}

/// Stateful small-file document with bounded reads and optional change watching.
pub struct FileDocument {
    view: FileView,
    data: Option<Vec<u8>>,
    error: Option<FileViewError>,
    watcher: Option<FileWatcher>,
    maximum: usize,
    atomic_writes: bool,
}

impl FileDocument {
    pub fn new(path: impl Into<PathBuf>, maximum: usize) -> Self {
        Self {
            view: FileView::new(path),
            data: None,
            error: None,
            watcher: None,
            maximum,
            atomic_writes: true,
        }
    }

    pub fn path(&self) -> &Path {
        self.view.path()
    }

    pub fn set_path(&mut self, path: impl Into<PathBuf>) {
        self.view = FileView::new(path);
        self.data = None;
        self.error = None;
        self.watcher = None;
    }

    pub fn loaded(&self) -> bool {
        self.data.is_some()
    }

    pub fn exists(&self) -> bool {
        self.view.exists()
    }

    pub fn error(&self) -> Option<FileViewError> {
        self.error
    }

    pub fn data(&self) -> Option<&[u8]> {
        self.data.as_deref()
    }

    pub fn text(&self) -> Option<String> {
        self.data
            .as_ref()
            .map(|data| String::from_utf8_lossy(data).into_owned())
    }

    pub fn reload(&mut self) -> bool {
        match self.view.read_bounded(self.maximum) {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                true
            }
            Err(error) => {
                self.data = None;
                self.error = Some(classify_file_error(&error));
                false
            }
        }
    }

    pub fn set_atomic_writes(&mut self, atomic: bool) {
        self.atomic_writes = atomic;
    }

    pub fn atomic_writes(&self) -> bool {
        self.atomic_writes
    }

    pub fn set_data(&mut self, data: &[u8]) -> bool {
        if data.len() > self.maximum {
            self.error = Some(FileViewError::TooLarge);
            return false;
        }
        match self.view.write_with_mode(data, self.atomic_writes) {
            Ok(()) => {
                self.data = Some(data.to_vec());
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(classify_file_error(&error));
                false
            }
        }
    }

    pub fn set_watch_changes(&mut self, enabled: bool) -> io::Result<()> {
        self.watcher = if enabled {
            Some(self.view.watch()?)
        } else {
            None
        };
        Ok(())
    }

    pub fn watch_changes(&self) -> bool {
        self.watcher.is_some()
    }

    pub fn next_change(&self, timeout: Duration) -> Option<FileEvent> {
        self.watcher
            .as_ref()
            .and_then(|watcher| watcher.next_event(timeout))
    }
}

/// Filesystem change reported by inotify.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileEvent {
    Changed,
    Moved,
    Deleted,
}

/// Inotify-backed file event receiver.
pub struct FileWatcher {
    events: mpsc::Receiver<FileEvent>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl FileWatcher {
    fn new(path: &Path) -> io::Result<Self> {
        let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no name"))?
            .as_bytes()
            .to_vec();
        inotify::add_watch(
            &fd,
            parent,
            WatchFlags::MODIFY
                | WatchFlags::CLOSE_WRITE
                | WatchFlags::MOVED_TO
                | WatchFlags::CREATE
                | WatchFlags::DELETE,
        )?;
        let (tx, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let join = thread::spawn(move || {
            let mut buffer = [MaybeUninit::uninit(); 4096];
            let mut reader = inotify::Reader::new(fd, &mut buffer);
            while !worker_stop.load(Ordering::Acquire) {
                match reader.next() {
                    Ok(event) => {
                        if event.file_name().map(|value| value.to_bytes()) != Some(name.as_slice())
                        {
                            continue;
                        }
                        let flags = event.events();
                        let event = if flags.intersects(ReadFlags::DELETE) {
                            FileEvent::Deleted
                        } else if flags.intersects(ReadFlags::MOVED_TO) {
                            FileEvent::Moved
                        } else {
                            FileEvent::Changed
                        };
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                    Err(Errno::AGAIN) => thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            events,
            stop,
            join: Some(join),
        })
    }

    pub fn next_event(&self, timeout: Duration) -> Option<FileEvent> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Connected Unix-domain byte stream.
pub struct Socket(UnixStream);

impl Socket {
    pub fn pair() -> io::Result<(Self, Self)> {
        UnixStream::pair().map(|(left, right)| (Self(left), Self(right)))
    }

    pub fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        UnixStream::connect(path).map(Self)
    }

    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    pub fn receive(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }

    /// Receives bytes with a temporary read timeout.
    pub fn receive_timeout(&mut self, bytes: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.0.set_read_timeout(Some(timeout))?;
        let result = self.0.read(bytes);
        let _ = self.0.set_read_timeout(None);
        result
    }
}

/// Listening Unix-domain socket.
pub struct SocketServer(UnixListener);

impl SocketServer {
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        UnixListener::bind(path).map(Self)
    }

    pub fn accept(&self) -> io::Result<Socket> {
        self.0.accept().map(|(stream, _)| Socket(stream))
    }

    /// Accepts one pending client without blocking the caller.
    pub fn try_accept(&self) -> io::Result<Option<Socket>> {
        self.0.set_nonblocking(true)?;
        let result = match self.0.accept() {
            Ok((stream, _)) => Ok(Some(Socket(stream))),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        };
        let reset = self.0.set_nonblocking(false);
        match (result, reset) {
            (Ok(socket), Ok(())) => Ok(socket),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }
}

const IPC_MAX_CONNECTIONS: usize = 32;
const IPC_MAX_REQUEST: usize = 64 * 1024;
const IPC_MAX_RESPONSE: usize = 256 * 1024;
const IPC_TIMEOUT: Duration = Duration::from_secs(2);

/// Primitive value carried by the public IPC protocol.
#[derive(Clone, Debug, PartialEq)]
pub enum IpcValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
}

/// One decoded IPC operation.
#[derive(Clone, Debug, PartialEq)]
pub enum IpcRequest {
    Call { target: String, args: Vec<IpcValue> },
    Verbs,
    Log,
    Bindings,
    Kill,
}

/// Credentials read from the accepted Unix socket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

/// Reply returned to one IPC request.
#[derive(Clone, Debug, PartialEq)]
pub struct IpcReply {
    pub ok: bool,
    pub result: Vec<IpcValue>,
    pub error: Option<String>,
}

impl IpcReply {
    pub fn success(result: Vec<IpcValue>) -> Self {
        Self {
            ok: true,
            result,
            error: None,
        }
    }

    pub fn refused(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            result: Vec::new(),
            error: Some(error.into()),
        }
    }

    pub fn to_wire(&self) -> io::Result<Vec<u8>> {
        encode_ipc_reply(self)
    }
}

/// Request handed from socket workers to the shell event loop.
pub struct IpcIncoming {
    pub peer: PeerCredentials,
    pub request: IpcRequest,
    reply: mpsc::SyncSender<IpcReply>,
}

impl IpcIncoming {
    pub fn reply(self, reply: IpcReply) {
        let _ = self.reply.send(reply);
    }
}

/// Bounded persistent Unix-socket IPC server.
pub struct IpcServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn bind(path: impl Into<PathBuf>, requests: mpsc::Sender<IpcIncoming>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() {
            match UnixStream::connect(&path) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "IPC socket is already active",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    fs::remove_file(&path)?;
                }
                Err(error) => return Err(error),
            }
        }
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let active = Arc::new(AtomicUsize::new(0));
        let join = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if active.fetch_add(1, Ordering::AcqRel) >= IPC_MAX_CONNECTIONS {
                            active.fetch_sub(1, Ordering::AcqRel);
                            let _ =
                                write_ipc_reply(stream, IpcReply::refused("too many connections"));
                            continue;
                        }
                        let requests = requests.clone();
                        let active = Arc::clone(&active);
                        thread::spawn(move || {
                            serve_ipc_connection(stream, requests);
                            active.fetch_sub(1, Ordering::AcqRel);
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            path,
            stop,
            join: Some(join),
        })
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// Sends one request and reads one wire reply.
pub fn ipc_call(path: impl AsRef<Path>, request: &IpcRequest) -> io::Result<IpcReply> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(IPC_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_TIMEOUT))?;
    let mut request = encode_ipc_request(request)?;
    request.push(b'\n');
    stream.write_all(&request)?;
    let mut response = Vec::new();
    BufReader::new(stream)
        .take(IPC_MAX_RESPONSE as u64 + 1)
        .read_until(b'\n', &mut response)?;
    if response.len() > IPC_MAX_RESPONSE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC response exceeds size limit",
        ));
    }
    decode_ipc_reply(&response)
}

fn serve_ipc_connection(mut stream: UnixStream, requests: mpsc::Sender<IpcIncoming>) {
    if stream.set_read_timeout(Some(IPC_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(IPC_TIMEOUT)).is_err()
    {
        return;
    }
    let Ok(peer) = peer_credentials(&stream) else {
        return;
    };
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream);
    loop {
        let mut line = Vec::new();
        match reader
            .by_ref()
            .take(IPC_MAX_REQUEST as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => break,
            Ok(_) if line.len() > IPC_MAX_REQUEST => {
                let _ =
                    write_ipc_reply(&mut stream, IpcReply::refused("request exceeds size limit"));
                break;
            }
            Ok(_) => {}
        }
        let request = match decode_ipc_request(&line) {
            Ok(request) => request,
            Err(error) => {
                let _ = write_ipc_reply(&mut stream, IpcReply::refused(error.to_string()));
                continue;
            }
        };
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if requests
            .send(IpcIncoming {
                peer,
                request,
                reply: reply_tx,
            })
            .is_err()
        {
            break;
        }
        let reply = reply_rx
            .recv_timeout(IPC_TIMEOUT)
            .unwrap_or_else(|_| IpcReply::refused("request timed out"));
        if write_ipc_reply(&mut stream, reply).is_err() {
            break;
        }
    }
}

fn peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(credentials).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCredentials {
        pid: credentials.pid as u32,
        uid: credentials.uid,
        gid: credentials.gid,
    })
}

fn write_ipc_reply(mut stream: impl Write, reply: IpcReply) -> io::Result<()> {
    let mut bytes = encode_ipc_reply(&reply)?;
    if bytes.len() > IPC_MAX_RESPONSE {
        bytes = encode_ipc_reply(&IpcReply::refused("response exceeds size limit"))?;
    }
    bytes.push(b'\n');
    stream.write_all(&bytes)
}

fn encode_ipc_request(request: &IpcRequest) -> io::Result<Vec<u8>> {
    let value = match request {
        IpcRequest::Call { target, args } => serde_json::json!({
            "op": "call",
            "target": target,
            "args": args.iter().map(ipc_value_to_json).collect::<Vec<_>>(),
        }),
        IpcRequest::Verbs => serde_json::json!({ "op": "verbs" }),
        IpcRequest::Log => serde_json::json!({ "op": "log" }),
        IpcRequest::Bindings => serde_json::json!({ "op": "bindings" }),
        IpcRequest::Kill => serde_json::json!({ "op": "kill" }),
    };
    serde_json::to_vec(&value).map_err(io::Error::other)
}

fn decode_ipc_request(bytes: &[u8]) -> io::Result<IpcRequest> {
    let value: JsonValue = serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let object = value
        .as_object()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "request must be an object"))?;
    let op = object
        .get("op")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "request needs string op"))?;
    match op {
        "call" => {
            let target = object
                .get("target")
                .and_then(JsonValue::as_str)
                .filter(|target| !target.is_empty() && target.len() <= 256)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid call target"))?;
            let args = object
                .get("args")
                .and_then(JsonValue::as_array)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "call args must be an array")
                })?;
            if args.len() > 64 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "too many call args",
                ));
            }
            Ok(IpcRequest::Call {
                target: target.to_owned(),
                args: args
                    .iter()
                    .map(ipc_value_from_json)
                    .collect::<io::Result<_>>()?,
            })
        }
        "verbs" => Ok(IpcRequest::Verbs),
        "log" => Ok(IpcRequest::Log),
        "bindings" => Ok(IpcRequest::Bindings),
        "kill" => Ok(IpcRequest::Kill),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown IPC operation",
        )),
    }
}

fn encode_ipc_reply(reply: &IpcReply) -> io::Result<Vec<u8>> {
    let mut object = JsonMap::new();
    object.insert("ok".into(), JsonValue::Bool(reply.ok));
    object.insert("n".into(), JsonValue::from(reply.result.len()));
    object.insert(
        "result".into(),
        JsonValue::Array(reply.result.iter().map(ipc_value_to_json).collect()),
    );
    if let Some(error) = &reply.error {
        object.insert("error".into(), JsonValue::String(error.clone()));
    }
    serde_json::to_vec(&JsonValue::Object(object)).map_err(io::Error::other)
}

fn decode_ipc_reply(bytes: &[u8]) -> io::Result<IpcReply> {
    let value: JsonValue = serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let object = value
        .as_object()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "reply must be an object"))?;
    let ok = object
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "reply needs boolean ok"))?;
    let result = object
        .get("result")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "reply result must be an array"))?
        .iter()
        .map(ipc_value_from_json)
        .collect::<io::Result<Vec<_>>>()?;
    let error = object
        .get("error")
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "reply error must be a string")
            })
        })
        .transpose()?;
    Ok(IpcReply { ok, result, error })
}

fn ipc_value_to_json(value: &IpcValue) -> JsonValue {
    match value {
        IpcValue::Nil => JsonValue::Null,
        IpcValue::Boolean(value) => JsonValue::Bool(*value),
        IpcValue::Integer(value) => JsonValue::from(*value),
        IpcValue::Number(value) => JsonValue::from(*value),
        IpcValue::String(value) => JsonValue::String(value.clone()),
    }
}

fn ipc_value_from_json(value: &JsonValue) -> io::Result<IpcValue> {
    match value {
        JsonValue::Null => Ok(IpcValue::Nil),
        JsonValue::Bool(value) => Ok(IpcValue::Boolean(*value)),
        JsonValue::Number(value) if value.is_i64() => {
            Ok(IpcValue::Integer(value.as_i64().unwrap()))
        }
        JsonValue::Number(value) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(IpcValue::Number)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid numeric IPC value")),
        JsonValue::String(value) => Ok(IpcValue::String(value.clone())),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC values must be primitive",
        )),
    }
}

/// Periodic timer event receiver.
pub struct Timer {
    ticks: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Timer {
    pub fn every(interval: Duration) -> io::Result<Self> {
        if interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "timer interval cannot be zero",
            ));
        }
        let (tx, ticks) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let join = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                thread::park_timeout(interval);
                let _ = tx.try_send(());
            }
        });
        Ok(Self {
            ticks,
            stop,
            join: Some(join),
        })
    }

    pub fn tick(&self, timeout: Duration) -> bool {
        self.ticks.recv_timeout(timeout).is_ok()
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.thread().unpark();
            let _ = join.join();
        }
    }
}

/// Message bus used by a generic D-Bus proxy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bus {
    Session,
    System,
}

/// Typed generic D-Bus method and property client.
#[derive(Clone, Debug)]
pub struct DbusProxy {
    proxy: ZbusProxy<'static>,
    bus: Bus,
    destination: String,
    path: String,
    interface: String,
}

/// Bounded value transferable through the Lua D-Bus facade.
#[derive(Clone, Debug, PartialEq)]
pub enum DbusValue {
    Nil,
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Number(f64),
    String(String),
    List(Vec<DbusValue>),
    Map(BTreeMap<String, DbusValue>),
    Typed {
        signature: String,
        value: Box<DbusValue>,
    },
}

impl DbusProxy {
    /// Connects a proxy to one bus object and interface.
    pub fn connect(
        bus: Bus,
        destination: impl Into<String>,
        path: impl Into<String>,
        interface: impl Into<String>,
    ) -> zbus::Result<Self> {
        let connection = match bus {
            Bus::Session => DbusConnection::session()?,
            Bus::System => DbusConnection::system()?,
        };
        let destination = destination.into();
        let path = path.into();
        let interface = interface.into();
        let proxy = ZbusProxy::new_owned(
            connection,
            destination.clone(),
            path.clone(),
            interface.clone(),
        )?;
        Ok(Self {
            proxy,
            bus,
            destination,
            path,
            interface,
        })
    }

    /// Returns the connection's unique bus name.
    pub fn unique_name(&self) -> Option<String> {
        self.proxy
            .connection()
            .unique_name()
            .map(ToString::to_string)
    }

    /// Calls one method and deserializes its reply body.
    pub fn call<B, R>(&self, method: &str, body: &B) -> zbus::Result<R>
    where
        B: Serialize + DynamicType,
        R: for<'de> DynamicDeserialize<'de>,
    {
        self.proxy.call(method, body)
    }

    /// Reads one remote property.
    pub fn get_property<T>(&self, property: &str) -> zbus::Result<T>
    where
        T: TryFrom<OwnedValue>,
        T::Error: Into<zbus::Error>,
    {
        self.proxy.get_property(property)
    }

    /// Writes one remote property.
    pub fn set_property<'value, T>(&self, property: &str, value: T) -> zbus::Result<()>
    where
        T: 'value + Into<Value<'value>>,
    {
        Ok(self.proxy.set_property(property, value)?)
    }

    /// Returns the remote object's introspection XML.
    pub fn introspect(&self) -> zbus::Result<String> {
        Ok(self.proxy.introspect()?)
    }

    /// Reads one property for an interpreter-facing facade.
    pub fn get_value(&self, property: &str) -> Result<DbusValue, String> {
        let value: OwnedValue = self
            .proxy
            .get_property(property)
            .map_err(|error| error.to_string())?;
        basic_value(&value)
    }

    /// Calls a no-argument method returning a supported value.
    pub fn call_value(&self, method: &str) -> Result<DbusValue, String> {
        let message = self
            .proxy
            .call_method(method, &())
            .map_err(|error| error.to_string())?;
        decode_message_value(&message)
    }

    /// Calls a method with one scalar or a list of positional scalar arguments.
    pub fn call_value_with(&self, method: &str, value: &DbusValue) -> Result<DbusValue, String> {
        let message = match value {
            DbusValue::Nil => self.proxy.call_method(method, &()),
            DbusValue::Bool(value) => self.proxy.call_method(method, &(*value,)),
            DbusValue::Integer(value) => self.proxy.call_method(method, &(*value,)),
            DbusValue::Unsigned(value) => self.proxy.call_method(method, &(*value,)),
            DbusValue::Number(value) => self.proxy.call_method(method, &(*value,)),
            DbusValue::String(value) => self.proxy.call_method(method, &(value.as_str(),)),
            DbusValue::Typed { .. } => {
                let body = StructureBuilder::new()
                    .append_field(dbus_argument_value(value)?)
                    .build()
                    .map_err(|error| error.to_string())?;
                self.proxy.call_method(method, &body)
            }
            DbusValue::List(values) if values.is_empty() => self.proxy.call_method(method, &()),
            DbusValue::List(values) => {
                let mut body = StructureBuilder::new();
                for value in values {
                    body = body.append_field(dbus_argument_value(value)?);
                }
                let body = body.build().map_err(|error| error.to_string())?;
                self.proxy.call_method(method, &body)
            }
            DbusValue::Map(_) => {
                return Err("D-Bus maps need an explicit signature".to_owned());
            }
        }
        .map_err(|error| error.to_string())?;
        decode_message_value(&message)
    }

    /// Writes one scalar property for an interpreter-facing facade.
    pub fn set_value(&self, property: &str, value: &DbusValue) -> Result<(), String> {
        let result = match value {
            DbusValue::Nil => return Err("D-Bus properties cannot be nil".to_owned()),
            DbusValue::Bool(value) => self.set_property(property, *value),
            DbusValue::Integer(value) => self.set_property(property, *value),
            DbusValue::Unsigned(value) => self.set_property(property, *value),
            DbusValue::Number(value) => self.set_property(property, *value),
            DbusValue::String(value) => self.set_property(property, value.as_str()),
            DbusValue::Typed { .. } => {
                let value = dbus_argument_value(value)?;
                self.set_property(property, value)
            }
            DbusValue::List(_) | DbusValue::Map(_) => {
                return Err("compound D-Bus properties are not supported".to_owned());
            }
        };
        result.map_err(|error| error.to_string())
    }

    /// Subscribes to one signal on a dedicated bus connection.
    pub fn subscribe(&self, signal: impl Into<String>) -> zbus::Result<DbusSignal> {
        let connection = match self.bus {
            Bus::Session => DbusConnection::session()?,
            Bus::System => DbusConnection::system()?,
        };
        let proxy = ZbusProxy::new_owned(
            connection.clone(),
            self.destination.clone(),
            self.path.clone(),
            self.interface.clone(),
        )?;
        let iterator = proxy.receive_signal(signal.into())?;
        let (tx, events) = mpsc::channel();
        let join = thread::spawn(move || {
            for message in iterator {
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        Ok(DbusSignal {
            events,
            connection: Some(connection),
            join: Some(join),
        })
    }
}

fn dbus_argument_value(value: &DbusValue) -> Result<Value<'_>, String> {
    match value {
        DbusValue::Bool(value) => Ok(Value::Bool(*value)),
        DbusValue::Integer(value) => Ok(Value::I64(*value)),
        DbusValue::Unsigned(value) => Ok(Value::U64(*value)),
        DbusValue::Number(value) => Ok(Value::F64(*value)),
        DbusValue::String(value) => Ok(Value::Str(value.as_str().into())),
        DbusValue::Typed { signature, value } => typed_dbus_value(signature, value),
        DbusValue::Nil => Err("nil cannot be a positional D-Bus argument".to_owned()),
        DbusValue::List(_) | DbusValue::Map(_) => {
            Err("nested D-Bus arguments need an explicit signature".to_owned())
        }
    }
}

fn typed_dbus_value<'a>(signature: &str, value: &'a DbusValue) -> Result<Value<'a>, String> {
    let signature = Signature::try_from(signature)
        .map_err(|error| format!("invalid D-Bus signature: {error}"))?;
    dbus_value_for_signature(&signature, value)
}

fn dbus_value_for_signature<'a>(
    signature: &Signature,
    value: &'a DbusValue,
) -> Result<Value<'a>, String> {
    let name = signature.to_string();
    let integer = || match value {
        DbusValue::Integer(value) => Ok(i128::from(*value)),
        DbusValue::Unsigned(value) => Ok(i128::from(*value)),
        _ => Err(format!("D-Bus `{name}` value must be an integer")),
    };
    let range_error = || format!("D-Bus `{name}` integer is out of range");
    Ok(match signature {
        Signature::U8 => Value::U8(u8::try_from(integer()?).map_err(|_| range_error())?),
        Signature::I16 => Value::I16(i16::try_from(integer()?).map_err(|_| range_error())?),
        Signature::U16 => Value::U16(u16::try_from(integer()?).map_err(|_| range_error())?),
        Signature::I32 => Value::I32(i32::try_from(integer()?).map_err(|_| range_error())?),
        Signature::U32 => Value::U32(u32::try_from(integer()?).map_err(|_| range_error())?),
        Signature::I64 => Value::I64(i64::try_from(integer()?).map_err(|_| range_error())?),
        Signature::U64 => Value::U64(u64::try_from(integer()?).map_err(|_| range_error())?),
        Signature::F64 => match value {
            DbusValue::Number(value) => Value::F64(*value),
            DbusValue::Integer(value) => Value::F64(*value as f64),
            DbusValue::Unsigned(value) => Value::F64(*value as f64),
            _ => return Err("D-Bus `d` value must be numeric".to_owned()),
        },
        Signature::Bool => match value {
            DbusValue::Bool(value) => Value::Bool(*value),
            _ => return Err("D-Bus `b` value must be boolean".to_owned()),
        },
        Signature::Str => match value {
            DbusValue::String(value) => Value::Str(value.as_str().into()),
            _ => return Err("D-Bus `s` value must be a string".to_owned()),
        },
        Signature::ObjectPath => match value {
            DbusValue::String(value) => Value::ObjectPath(
                ObjectPath::try_from(value.as_str()).map_err(|error| error.to_string())?,
            ),
            _ => return Err("D-Bus `o` value must be a string".to_owned()),
        },
        Signature::Signature => match value {
            DbusValue::String(value) => Value::Signature(
                Signature::try_from(value.as_str()).map_err(|error| error.to_string())?,
            ),
            _ => return Err("D-Bus `g` value must be a string".to_owned()),
        },
        Signature::Variant => Value::Value(Box::new(inferred_dbus_value(value)?)),
        Signature::Array(child) => {
            let DbusValue::List(values) = value else {
                return Err(format!("D-Bus `{name}` value must be a list"));
            };
            let mut array = Array::new(child.signature());
            for value in values {
                array
                    .append(dbus_value_for_signature(child.signature(), value)?)
                    .map_err(|error| error.to_string())?;
            }
            Value::Array(array)
        }
        Signature::Dict {
            key: key_signature,
            value: value_signature,
        } => {
            let DbusValue::Map(values) = value else {
                return Err(format!("D-Bus `{name}` value must be a map"));
            };
            let mut dict = Dict::new(key_signature.signature(), value_signature.signature());
            for (key, value) in values {
                dict.append(
                    dbus_map_key(key_signature.signature(), key)?,
                    dbus_value_for_signature(value_signature.signature(), value)?,
                )
                .map_err(|error| error.to_string())?;
            }
            Value::Dict(dict)
        }
        Signature::Structure(fields) => {
            let DbusValue::List(values) = value else {
                return Err(format!("D-Bus `{name}` value must be a list"));
            };
            if values.len() != fields.len() {
                return Err(format!(
                    "D-Bus `{name}` needs {} fields, found {}",
                    fields.len(),
                    values.len()
                ));
            }
            let mut structure = StructureBuilder::new();
            for (field, value) in fields.iter().zip(values) {
                structure = structure.append_field(dbus_value_for_signature(field, value)?);
            }
            Value::Structure(structure.build().map_err(|error| error.to_string())?)
        }
        Signature::Unit => return Err("D-Bus unit values cannot be arguments".to_owned()),
        #[cfg(unix)]
        Signature::Fd => return Err("D-Bus file descriptors cannot come from Lua".to_owned()),
        #[allow(unreachable_patterns)]
        _ => return Err(format!("unsupported explicit D-Bus signature `{name}`")),
    })
}

fn dbus_map_key<'a>(signature: &Signature, key: &'a str) -> Result<Value<'a>, String> {
    match signature {
        Signature::Str => Ok(Value::Str(key.into())),
        Signature::ObjectPath => Ok(Value::ObjectPath(
            ObjectPath::try_from(key).map_err(|error| error.to_string())?,
        )),
        Signature::Signature => Ok(Value::Signature(
            Signature::try_from(key).map_err(|error| error.to_string())?,
        )),
        _ => Err(format!(
            "D-Bus map keys from Lua cannot use signature `{signature}`"
        )),
    }
}

fn inferred_dbus_value(value: &DbusValue) -> Result<Value<'_>, String> {
    match value {
        DbusValue::Typed { signature, value } => typed_dbus_value(signature, value),
        DbusValue::Bool(value) => Ok(Value::Bool(*value)),
        DbusValue::Integer(value) => Ok(Value::I64(*value)),
        DbusValue::Unsigned(value) => Ok(Value::U64(*value)),
        DbusValue::Number(value) => Ok(Value::F64(*value)),
        DbusValue::String(value) => Ok(Value::Str(value.as_str().into())),
        DbusValue::Nil => Err("nil cannot be a D-Bus variant".to_owned()),
        DbusValue::List(_) | DbusValue::Map(_) => {
            Err("compound D-Bus variants need an explicit signature".to_owned())
        }
    }
}

fn decode_message_value(message: &zbus::Message) -> Result<DbusValue, String> {
    let body = message.body();
    if body.deserialize::<()>().is_ok() {
        return Ok(DbusValue::Nil);
    }
    if let Ok(value) = body.deserialize::<bool>() {
        return Ok(DbusValue::Bool(value));
    }
    if let Ok(value) = body.deserialize::<i16>() {
        return Ok(DbusValue::Integer(value as i64));
    }
    if let Ok(value) = body.deserialize::<i32>() {
        return Ok(DbusValue::Integer(value as i64));
    }
    if let Ok(value) = body.deserialize::<i64>() {
        return Ok(DbusValue::Integer(value));
    }
    if let Ok(value) = body.deserialize::<u8>() {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = body.deserialize::<u16>() {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = body.deserialize::<u32>() {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = body.deserialize::<u64>() {
        return Ok(DbusValue::Unsigned(value));
    }
    if let Ok(value) = body.deserialize::<f64>() {
        return Ok(DbusValue::Number(value));
    }
    if let Ok(value) = body.deserialize::<String>() {
        return Ok(DbusValue::String(value));
    }
    if let Ok(value) = body.deserialize::<Structure<'_>>() {
        return structure_value(&value);
    }
    if let Ok(value) = body.deserialize::<Array<'_>>() {
        return array_value(&value);
    }
    Err("D-Bus reply type is not supported".to_owned())
}

fn dynamic_value(value: &Value<'_>) -> Result<DbusValue, String> {
    Ok(match value {
        Value::U8(value) => DbusValue::Unsigned(u64::from(*value)),
        Value::Bool(value) => DbusValue::Bool(*value),
        Value::I16(value) => DbusValue::Integer(i64::from(*value)),
        Value::U16(value) => DbusValue::Unsigned(u64::from(*value)),
        Value::I32(value) => DbusValue::Integer(i64::from(*value)),
        Value::U32(value) => DbusValue::Unsigned(u64::from(*value)),
        Value::I64(value) => DbusValue::Integer(*value),
        Value::U64(value) => DbusValue::Unsigned(*value),
        Value::F64(value) => DbusValue::Number(*value),
        Value::Str(value) => DbusValue::String(value.to_string()),
        Value::Signature(value) => DbusValue::String(value.to_string()),
        Value::ObjectPath(value) => DbusValue::String(value.to_string()),
        Value::Value(value) => dynamic_value(value)?,
        Value::Array(value) => array_value(value)?,
        Value::Dict(value) => dict_value(value)?,
        Value::Structure(value) => structure_value(value)?,
        #[cfg(unix)]
        Value::Fd(_) => return Err("D-Bus file descriptors cannot cross into Lua".to_owned()),
        #[allow(unreachable_patterns)]
        _ => return Err("D-Bus value is not supported".to_owned()),
    })
}

fn structure_value(value: &Structure<'_>) -> Result<DbusValue, String> {
    value
        .fields()
        .iter()
        .map(dynamic_value)
        .collect::<Result<Vec<_>, _>>()
        .map(DbusValue::List)
}

fn array_value(value: &Array<'_>) -> Result<DbusValue, String> {
    value
        .inner()
        .iter()
        .map(dynamic_value)
        .collect::<Result<Vec<_>, _>>()
        .map(DbusValue::List)
}

fn dict_value(value: &Dict<'_, '_>) -> Result<DbusValue, String> {
    let mut map = BTreeMap::new();
    for (key, value) in value.iter() {
        let key = match dynamic_value(key)? {
            DbusValue::String(key) => key,
            _ => return Err("D-Bus dictionary keys must be strings".to_owned()),
        };
        map.insert(key, dynamic_value(value)?);
    }
    Ok(DbusValue::Map(map))
}

/// Blocking receiver for a filtered D-Bus signal stream.
pub struct DbusSignal {
    events: mpsc::Receiver<zbus::Message>,
    connection: Option<DbusConnection>,
    join: Option<JoinHandle<()>>,
}

impl DbusSignal {
    /// Waits for the next signal message.
    pub fn next(&self, timeout: Duration) -> Option<zbus::Message> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Waits for and decodes the next scalar signal body.
    pub fn next_value(&self, timeout: Duration) -> Option<Result<DbusValue, String>> {
        self.next(timeout)
            .map(|message| decode_message_value(&message))
    }
}

impl Drop for DbusSignal {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.close();
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn basic_value(value: &OwnedValue) -> Result<DbusValue, String> {
    if matches!(
        &**value,
        Value::Array(_) | Value::Dict(_) | Value::Structure(_) | Value::Value(_)
    ) {
        return dynamic_value(value);
    }
    if let Ok(value) = bool::try_from(value) {
        return Ok(DbusValue::Bool(value));
    }
    if let Ok(value) = i16::try_from(value) {
        return Ok(DbusValue::Integer(value as i64));
    }
    if let Ok(value) = i32::try_from(value) {
        return Ok(DbusValue::Integer(value as i64));
    }
    if let Ok(value) = i64::try_from(value) {
        return Ok(DbusValue::Integer(value));
    }
    if let Ok(value) = u8::try_from(value) {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = u16::try_from(value) {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = u32::try_from(value) {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = u64::try_from(value) {
        return Ok(DbusValue::Unsigned(value));
    }
    if let Ok(value) = f64::try_from(value) {
        return Ok(DbusValue::Number(value));
    }
    if let Ok(value) = <&str>::try_from(value) {
        return Ok(DbusValue::String(value.to_owned()));
    }
    Err("D-Bus value is not a supported scalar".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_parser_keeps_partial_chunks() {
        let mut parser = LineParser::default();
        assert_eq!(parser.push(b"one\ntw"), ["one"]);
        assert_eq!(parser.push(b"o\r\n"), ["two"]);
        assert_eq!(parser.finish(), None);
    }

    #[test]
    fn split_parser_handles_multibyte_delimiters() {
        let mut parser = SplitParser::new(b"--".to_vec()).unwrap();
        assert_eq!(parser.push(b"a-b--c--"), [b"a-b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn file_document_tracks_load_write_and_errors() {
        let path = std::env::temp_dir().join(format!("mold-file-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut file = FileDocument::new(&path, 16);
        assert!(!file.reload());
        assert_eq!(file.error(), Some(FileViewError::FileNotFound));
        assert!(file.set_data(b"hello"));
        assert!(file.loaded());
        assert!(file.exists());
        assert_eq!(file.text().as_deref(), Some("hello"));
        file.set_atomic_writes(false);
        assert!(!file.atomic_writes());
        assert!(file.set_data(b"world"));
        assert_eq!(std::fs::read(&path).unwrap(), b"world");
        assert!(!file.set_data(b"this value is too large"));
        assert_eq!(file.error(), Some(FileViewError::TooLarge));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn socket_server_accepts_without_blocking() {
        let path = std::env::temp_dir().join(format!("mold-io-server-{}", std::process::id()));
        let server = SocketServer::bind(&path).unwrap();
        assert!(server.try_accept().unwrap().is_none());
        let _client = Socket::connect(&path).unwrap();
        assert!(server.try_accept().unwrap().is_some());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn process_streams_output_and_exit() {
        let mut process = Process::spawn("sh", ["-c", "printf out; printf err >&2"]).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let status = loop {
            match process.next_event(Duration::from_secs(1)).unwrap() {
                Some(ProcessEvent::Stdout(bytes)) => stdout.extend(bytes),
                Some(ProcessEvent::Stderr(bytes)) => stderr.extend(bytes),
                Some(ProcessEvent::Exit(status)) => break status,
                None if std::time::Instant::now() < deadline => {}
                None => panic!("child did not exit"),
            }
        };
        assert!(status.success());
        assert_eq!(stdout, b"out");
        assert_eq!(stderr, b"err");
    }

    #[test]
    fn process_config_applies_directory_and_environment() {
        let directory = std::env::temp_dir();
        let config = ProcessConfig {
            command: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s:%s' \"$PWD\" \"$MOLD_PROCESS_TEST\"".into(),
            ],
            environment: BTreeMap::from([("MOLD_PROCESS_TEST".into(), "ok".into())]),
            clear_environment: false,
            working_directory: Some(directory.clone()),
        };
        let mut process = Process::spawn_config(&config).unwrap();
        let mut stdout = Vec::new();
        loop {
            match process.next_event(Duration::from_secs(1)).unwrap() {
                Some(ProcessEvent::Stdout(bytes)) => stdout.extend(bytes),
                Some(ProcessEvent::Exit(status)) => {
                    assert!(status.success());
                    break;
                }
                Some(ProcessEvent::Stderr(_)) | None => {}
            }
        }
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!("{}:ok", directory.display())
        );
    }

    #[test]
    fn file_view_writes_and_watches_atomically() {
        let path = std::env::temp_dir().join(format!("mold-io-{}", std::process::id()));
        fs::write(&path, b"old").unwrap();
        let file = FileView::new(&path);
        let watcher = file.watch().unwrap();
        file.write(b"new").unwrap();
        assert_eq!(file.read().unwrap(), b"new");
        assert!(watcher.next_event(Duration::from_secs(1)).is_some());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn timer_emits_and_stops() {
        let timer = Timer::every(Duration::from_millis(10)).unwrap();
        assert!(timer.tick(Duration::from_secs(1)));
        drop(timer);
    }

    #[test]
    fn ipc_server_keeps_connections_open_and_reads_peer_identity() {
        let path = std::env::temp_dir().join(format!("mold-ipc-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        let (tx, rx) = mpsc::channel();
        let server = IpcServer::bind(&path, tx).unwrap();
        let responder = thread::spawn(move || {
            for _ in 0..3 {
                let incoming = rx.recv_timeout(Duration::from_secs(1)).unwrap();
                assert_eq!(incoming.peer.pid, std::process::id());
                incoming.reply(IpcReply::success(vec![IpcValue::String("ok".into())]));
            }
        });
        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        for request in [IpcRequest::Verbs, IpcRequest::Log, IpcRequest::Bindings] {
            let mut bytes = encode_ipc_request(&request).unwrap();
            bytes.push(b'\n');
            stream.write_all(&bytes).unwrap();
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            assert_eq!(
                decode_ipc_reply(reply.as_bytes()).unwrap(),
                IpcReply::success(vec![IpcValue::String("ok".into())])
            );
        }
        responder.join().unwrap();
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn ipc_server_reclaims_stale_socket_paths() {
        let path = std::env::temp_dir().join(format!("mold-ipc-stale-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        drop(listener);
        let (tx, _rx) = mpsc::channel();
        let server = IpcServer::bind(&path, tx).unwrap();
        assert!(path.exists());
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn ipc_wire_refuses_nested_values_and_keeps_result_count() {
        assert!(decode_ipc_request(br#"{"op":"call","target":"x","args":[{}]}"#).is_err());
        let bytes = encode_ipc_reply(&IpcReply::success(vec![
            IpcValue::Integer(7),
            IpcValue::Boolean(true),
        ]))
        .unwrap();
        let value: JsonValue = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["n"], 2);
        assert_eq!(value["result"], serde_json::json!([7, true]));
    }

    #[test]
    fn dbus_argument_lists_build_positional_structures() {
        let arguments = [
            DbusValue::String("device".to_owned()),
            DbusValue::Typed {
                signature: "u".to_owned(),
                value: Box::new(DbusValue::Integer(7)),
            },
            DbusValue::Bool(true),
        ];
        let mut body = StructureBuilder::new();
        for argument in &arguments {
            body = body.append_field(dbus_argument_value(argument).unwrap());
        }
        let body = body.build().unwrap();

        assert_eq!(body.fields().len(), 3);
        assert!(matches!(&body.fields()[0], Value::Str(value) if value.as_str() == "device"));
        assert!(matches!(body.fields()[1], Value::U32(7)));
        assert!(matches!(body.fields()[2], Value::Bool(true)));
    }

    #[test]
    fn explicit_dbus_signatures_build_compound_values() {
        let array_value = DbusValue::List(vec![
            DbusValue::String("one".into()),
            DbusValue::String("two".into()),
        ]);
        let array = typed_dbus_value("as", &array_value).unwrap();
        assert_eq!(array.value_signature().to_string(), "as");

        let map_value = DbusValue::Map(BTreeMap::from([
            ("enabled".into(), DbusValue::Bool(true)),
            (
                "count".into(),
                DbusValue::Typed {
                    signature: "u".into(),
                    value: Box::new(DbusValue::Integer(7)),
                },
            ),
        ]));
        let map = typed_dbus_value("a{sv}", &map_value).unwrap();
        assert_eq!(map.value_signature().to_string(), "a{sv}");
        let decoded = dynamic_value(&map).unwrap();
        let DbusValue::Map(decoded) = decoded else {
            panic!("D-Bus dictionary did not decode as a map");
        };
        assert_eq!(decoded["enabled"], DbusValue::Bool(true));
        assert_eq!(decoded["count"], DbusValue::Unsigned(7));

        let structure_value = DbusValue::List(vec![
            DbusValue::String("name".into()),
            DbusValue::Integer(-2),
        ]);
        let structure = typed_dbus_value("(si)", &structure_value).unwrap();
        assert_eq!(structure.value_signature().to_string(), "(si)");
    }

    #[test]
    fn stream_collector_controls_publication_and_bounds() {
        let mut delayed = StreamCollector::new(8, true).unwrap();
        assert!(!delayed.push(b"ab").unwrap());
        assert_eq!(delayed.data(), b"");
        assert!(delayed.finish());
        assert_eq!(delayed.text(), "ab");

        let mut live = StreamCollector::new(4, false).unwrap();
        assert!(live.push(b"ab").unwrap());
        assert_eq!(live.data(), b"ab");
        assert!(live.push(b"cde").is_err());
        live.reset();
        assert!(!live.finished());
    }
}
