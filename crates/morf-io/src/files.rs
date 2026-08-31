use std::fmt;
use std::fs;
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;

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
    ///
    /// The two refusals this makes itself are returned as themselves. They used
    /// to be `io::Error`s carrying a message that the classifier a few lines
    /// below compared as a *string* — so the reason a read failed travelled
    /// from one function to another through prose, and a reworded message would
    /// have silently become `Unknown`.
    pub fn read_bounded(&self, maximum: usize) -> Result<Vec<u8>, FileViewError> {
        let metadata = fs::metadata(&self.path).map_err(|error| classify_file_error(&error))?;
        if metadata.is_dir() || !metadata.is_file() {
            return Err(FileViewError::NotAFile);
        }
        if metadata.len() > maximum as u64 {
            return Err(FileViewError::TooLarge);
        }
        self.read().map_err(|error| classify_file_error(&error))
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
                || "morf-tmp".to_owned(),
                |value| format!("{value}.morf-tmp"),
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

impl fmt::Display for FileViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
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

/// Classifies an error the operating system produced.
///
/// Only those: what this module refuses itself is returned as a
/// `FileViewError` directly, rather than wrapped in an `io::Error` whose
/// message this function would then have to read back.
fn classify_file_error(error: &io::Error) -> FileViewError {
    match error.kind() {
        io::ErrorKind::NotFound => FileViewError::FileNotFound,
        io::ErrorKind::PermissionDenied => FileViewError::PermissionDenied,
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
    watch_changes: bool,
    maximum: usize,
    preload: bool,
    atomic_writes: bool,
}

impl FileDocument {
    pub fn new(path: impl Into<PathBuf>, maximum: usize) -> Self {
        Self {
            view: FileView::new(path),
            data: None,
            error: None,
            watcher: None,
            watch_changes: false,
            maximum,
            preload: true,
            atomic_writes: true,
        }
    }

    pub fn path(&self) -> &Path {
        self.view.path()
    }

    pub fn set_path(&mut self, path: impl Into<PathBuf>) -> io::Result<()> {
        let view = FileView::new(path);
        let watcher = if self.watch_changes && !view.path().as_os_str().is_empty() {
            Some(view.watch()?)
        } else {
            None
        };
        self.view = view;
        self.data = None;
        self.error = None;
        self.watcher = watcher;
        Ok(())
    }

    pub fn set_preload(&mut self, preload: bool) {
        self.preload = preload;
    }

    pub fn preload(&self) -> bool {
        self.preload
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
                self.error = Some(error);
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
        self.watcher = if enabled && !self.view.path().as_os_str().is_empty() {
            Some(self.view.watch()?)
        } else {
            None
        };
        self.watch_changes = enabled;
        Ok(())
    }

    pub fn watch_changes(&self) -> bool {
        self.watch_changes
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
