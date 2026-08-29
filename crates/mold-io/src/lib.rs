//! Bounded process, file, socket, and timer primitives for mold.

use std::fs;
use std::io::{self, Read, Write};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;
use serde::Serialize;
use zbus::blocking::{Connection as DbusConnection, Proxy as ZbusProxy};
use zbus::zvariant::{DynamicDeserialize, DynamicType, OwnedValue, Value};

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

impl Process {
    /// Spawns a child without invoking a shell.
    pub fn spawn<I, S>(program: impl AsRef<std::ffi::OsStr>, args: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut child = Command::new(program)
            .args(args)
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

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
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

/// Scalar value transferable through the Lua D-Bus facade.
#[derive(Clone, Debug, PartialEq)]
pub enum DbusValue {
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Number(f64),
    String(String),
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

    /// Reads one scalar property for an interpreter-facing facade.
    pub fn get_value(&self, property: &str) -> Result<DbusValue, String> {
        let value: OwnedValue = self
            .proxy
            .get_property(property)
            .map_err(|error| error.to_string())?;
        basic_value(&value)
    }

    /// Calls a no-argument method returning one scalar value.
    pub fn call_value(&self, method: &str) -> Result<DbusValue, String> {
        let message = self
            .proxy
            .call_method(method, &())
            .map_err(|error| error.to_string())?;
        let body = message.body();
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
        Err("D-Bus reply is not a supported scalar".to_owned())
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
}
