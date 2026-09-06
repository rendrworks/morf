//! Linux-PAM, driven from a shell.
//!
//! A transaction runs on its own thread, because PAM is synchronous and blocks
//! inside modules for as long as they like — a fingerprint module waits for a
//! finger — and a shell cannot stop drawing while it does. What crosses back is
//! a stream of [`PamEvent`]s: every message a module sends, and finally the
//! verdict.
//!
//! Two ways to answer, one transaction. [`PamSession`] hands each prompt to
//! the caller and waits; [`PamAuthenticator`] answers from credentials given
//! up front. They share the conversation callback and everything else, so a
//! fix to one is a fix to both.

use std::error::Error as StdError;
use std::ffi::{CStr, CString, c_char, c_int};
use std::fmt;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use libloading::{Library, Symbol};

use crate::pam_conversation::{
    Answers, Bridge, PAM_CONV_ERR, PAM_SUCCESS, PamEvent, PamMessage, PamPrompt, PamResponse,
    conversation,
};

const PAM_DISALLOW_NULL_AUTHTOK: c_int = 1;

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct PamConversation {
    callback: Option<
        unsafe extern "C" fn(
            c_int,
            *const *const PamMessage,
            *mut *mut PamResponse,
            *mut std::ffi::c_void,
        ) -> c_int,
    >,
    data: *mut std::ffi::c_void,
}

/// A username and password, zeroed when dropped.
pub(crate) struct Credentials {
    pub(crate) username: CString,
    pub(crate) password: CString,
}

impl Drop for Credentials {
    fn drop(&mut self) {
        let length = self.password.as_bytes_with_nul().len();
        let pointer = self.password.as_ptr().cast_mut();
        for index in 0..length {
            unsafe { pointer.add(index).write_volatile(0) };
        }
    }
}

type PamStart = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const PamConversation,
    *mut *mut PamHandle,
) -> c_int;
type PamStartConfdir = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const PamConversation,
    *const c_char,
    *mut *mut PamHandle,
) -> c_int;
type PamAuthenticate = unsafe extern "C" fn(*mut PamHandle, c_int) -> c_int;
type PamAccount = unsafe extern "C" fn(*mut PamHandle, c_int) -> c_int;
type PamEnd = unsafe extern "C" fn(*mut PamHandle, c_int) -> c_int;
type PamStrerror = unsafe extern "C" fn(*mut PamHandle, c_int) -> *const c_char;

/// PAM transaction failure without credential contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PamError {
    code: Option<i32>,
    message: String,
}

impl PamError {
    pub fn code(&self) -> Option<i32> {
        self.code
    }

    fn plain(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for PamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for PamError {}

/// The code for a transaction the caller gave up on.
///
/// Reported as its own value rather than as the generic conversation error the
/// module sees, because a caller that cancelled wants to know its cancel took
/// and not that "the conversation failed".
pub const PAM_CANCELLED: i32 = -1;

/// One running PAM transaction, with the person on this end of it.
///
/// Dropping it cancels: the transaction thread gets a conversation error at
/// its next prompt and unwinds through `pam_end`. A module that is blocked in
/// its own wait — a sensor with no finger on it — is not interrupted by this,
/// because PAM has no way to interrupt it; the thread lingers until the module
/// gives up on its own, and then leaves.
pub struct PamSession {
    events: mpsc::Receiver<PamEvent>,
    answers: Option<mpsc::Sender<String>>,
    cancelled: Arc<AtomicBool>,
    finished: Option<Result<(), PamError>>,
    /// Whether a prompt has been handed out and not yet answered.
    ///
    /// `respond` is gated on this rather than on the channel being open,
    /// because the channel is open for the whole transaction and an answer
    /// sent while no question is pending would sit in it until the next one --
    /// which is how a password typed at "touch the sensor" ends up as the reply
    /// to "PIN:".
    awaiting: bool,
}

impl PamSession {
    /// Starts a transaction whose prompts the caller will answer.
    ///
    /// `confdir` names a directory of PAM service files to use instead of
    /// `/etc/pam.d`, so a shell can carry its own service definition rather
    /// than depend on one being installed. It needs Linux-PAM 1.4 or later; on
    /// an older libpam the transaction finishes at once with an error saying so.
    pub fn start(service: &str, username: &str, confdir: Option<&str>) -> Self {
        Self::launch(service, username, confdir, None)
    }

    /// Starts a transaction answered from credentials, for the password case.
    pub(crate) fn start_fixed(
        service: &str,
        username: &str,
        password: &str,
        confdir: Option<&str>,
    ) -> Self {
        Self::launch(service, username, confdir, Some(password))
    }

    fn launch(
        service: &str,
        username: &str,
        confdir: Option<&str>,
        password: Option<&str>,
    ) -> Self {
        let (events, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut session = Self {
            events: receiver,
            answers: None,
            cancelled: Arc::clone(&cancelled),
            finished: None,
            awaiting: false,
        };
        let prepared = (|| {
            let service = cstring("service", service)?;
            let user = cstring("username", username)?;
            let confdir = confdir.map(|dir| cstring("confdir", dir)).transpose()?;
            let answers = match password {
                Some(password) => Answers::Fixed(Credentials {
                    username: user.clone(),
                    password: cstring("password", password)?,
                }),
                None => {
                    let (sender, receiver) = mpsc::channel();
                    session.answers = Some(sender);
                    Answers::Interactive(receiver)
                }
            };
            Ok::<_, PamError>((service, user, confdir, answers))
        })();
        match prepared {
            Ok((service, user, confdir, answers)) => {
                let bridge = Bridge {
                    events: events.clone(),
                    answers,
                    cancelled,
                };
                thread::spawn(move || {
                    let outcome = transact(&service, &user, confdir.as_deref(), bridge);
                    let _ = events.send(PamEvent::Finished(outcome));
                });
            }
            // Nothing to run, but the caller is still owed a verdict, and
            // owed it through the same channel it will be reading.
            Err(error) => {
                let _ = events.send(PamEvent::Finished(Err(error)));
            }
        }
        session
    }

    /// The next thing the transaction has to say, within `timeout`.
    ///
    /// After `Finished` there is nothing more, and asking again returns the
    /// same verdict rather than blocking on a thread that has left.
    pub fn next(&mut self, timeout: Duration) -> Option<PamEvent> {
        if let Some(verdict) = &self.finished {
            return Some(PamEvent::Finished(verdict.clone()));
        }
        let event = self.events.recv_timeout(timeout).ok()?;
        match &event {
            PamEvent::Finished(verdict) => self.finished = Some(verdict.clone()),
            PamEvent::Message(PamPrompt::Prompt { .. }) => self.awaiting = true,
            PamEvent::Message(_) => {}
        }
        Some(event)
    }

    /// Answers the prompt the transaction is waiting on.
    ///
    /// `false` when there is nothing to answer: the transaction is fixed,
    /// finished, or cancelled. An answer nobody asked for is not queued for
    /// the next prompt — that is how a stale password ends up sent to a
    /// question about something else.
    pub fn respond(&mut self, answer: impl Into<String>) -> bool {
        if !self.awaiting {
            return false;
        }
        let sent = self
            .answers
            .as_ref()
            .is_some_and(|sender| sender.send(answer.into()).is_ok());
        self.awaiting = !sent;
        sent
    }

    /// Gives up. The transaction fails at its next prompt.
    pub fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        // Dropping the sender wakes a thread blocked on the next answer, and
        // the flag covers a thread that has not asked yet.
        self.answers = None;
        self.awaiting = false;
        if self.finished.is_none() {
            self.finished = Some(Err(PamError {
                code: Some(PAM_CANCELLED),
                message: "authentication cancelled".to_owned(),
            }));
        }
    }
}

/// Synchronous Linux-PAM authentication and account validation.
pub struct PamAuthenticator;

/// Authentication result produced away from the shell event loop.
pub struct PamTask {
    session: PamSession,
}

impl PamTask {
    /// Waits up to the supplied duration for PAM to finish.
    ///
    /// Informational messages are read and dropped on this path, as they were
    /// before the conversation could carry them; a caller that wants to see
    /// them uses a [`PamSession`].
    pub fn wait(&mut self, timeout: Duration) -> Option<Result<(), PamError>> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.session.next(remaining)? {
                PamEvent::Finished(verdict) => return Some(verdict),
                PamEvent::Message(_) => {}
            }
        }
    }
}

impl PamAuthenticator {
    /// Authenticates credentials through the named PAM service.
    pub fn authenticate(service: &str, username: &str, password: &str) -> Result<(), PamError> {
        let mut session = PamSession::start_fixed(service, username, password, None);
        loop {
            match session.next(Duration::from_secs(60 * 60)) {
                Some(PamEvent::Finished(verdict)) => return verdict,
                Some(PamEvent::Message(_)) => {}
                None => return Err(PamError::plain("PAM did not answer")),
            }
        }
    }

    /// Starts authentication on a dedicated worker thread.
    pub fn authenticate_async(
        service: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
        confdir: Option<&str>,
    ) -> PamTask {
        PamTask {
            session: PamSession::start_fixed(
                &service.into(),
                &username.into(),
                &password.into(),
                confdir,
            ),
        }
    }
}

fn cstring(field: &str, value: &str) -> Result<CString, PamError> {
    CString::new(value).map_err(|_| PamError::plain(format!("{field} contains a null byte")))
}

/// Runs one whole transaction on the calling thread.
fn transact(
    service: &CStr,
    username: &CStr,
    confdir: Option<&CStr>,
    bridge: Bridge,
) -> Result<(), PamError> {
    let library = load_pam()?;
    let authenticate: Symbol<'_, PamAuthenticate> =
        unsafe { symbol(&library, b"pam_authenticate\0")? };
    let account: Symbol<'_, PamAccount> = unsafe { symbol(&library, b"pam_acct_mgmt\0")? };
    let end: Symbol<'_, PamEnd> = unsafe { symbol(&library, b"pam_end\0")? };
    let strerror: Symbol<'_, PamStrerror> = unsafe { symbol(&library, b"pam_strerror\0")? };
    // Boxed so its address is stable for the whole transaction: PAM keeps the
    // pointer and calls back through it from inside every module.
    let bridge = Box::new(bridge);
    let conversation = PamConversation {
        callback: Some(conversation),
        data: ptr::from_ref(&*bridge).cast_mut().cast(),
    };
    let mut handle = ptr::null_mut();
    let mut status = match confdir {
        Some(confdir) => {
            // Optional in the library: absent before Linux-PAM 1.4, and a
            // caller who named a directory wants to know it was ignored rather
            // than have the system's service silently used instead.
            let start: Symbol<'_, PamStartConfdir> = unsafe { library.get(b"pam_start_confdir\0") }
                .map_err(|_| {
                    PamError::plain("this libpam has no pam_start_confdir; it needs Linux-PAM 1.4")
                })?;
            unsafe {
                start(
                    service.as_ptr(),
                    username.as_ptr(),
                    &conversation,
                    confdir.as_ptr(),
                    &mut handle,
                )
            }
        }
        None => {
            let start: Symbol<'_, PamStart> = unsafe { symbol(&library, b"pam_start\0")? };
            unsafe {
                start(
                    service.as_ptr(),
                    username.as_ptr(),
                    &conversation,
                    &mut handle,
                )
            }
        }
    };
    if status != PAM_SUCCESS {
        return Err(unsafe { pam_error(handle, status, &strerror) });
    }
    status = unsafe { authenticate(handle, PAM_DISALLOW_NULL_AUTHTOK) };
    if status == PAM_SUCCESS {
        status = unsafe { account(handle, PAM_DISALLOW_NULL_AUTHTOK) };
    }
    let result = if status == PAM_SUCCESS {
        Ok(())
    } else if bridge.cancelled.load(Ordering::SeqCst) && status == PAM_CONV_ERR {
        Err(PamError {
            code: Some(PAM_CANCELLED),
            message: "authentication cancelled".to_owned(),
        })
    } else {
        Err(unsafe { pam_error(handle, status, &strerror) })
    };
    let end_status = unsafe { end(handle, status) };
    if result.is_ok() && end_status != PAM_SUCCESS {
        return Err(unsafe { pam_error(ptr::null_mut(), end_status, &strerror) });
    }
    result
}

fn load_pam() -> Result<Library, PamError> {
    let architecture = std::env::consts::ARCH;
    let mut candidates = std::env::var_os("MORF_PAM_LIBRARY")
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    candidates.extend([
        // The loader's own search path first, then the two layouts distributions
        // actually use: Debian's multiarch directories and everyone else's flat
        // /usr/lib. Trying only the multiarch pair means no authentication at all
        // on Arch, Fedora or SUSE.
        std::path::PathBuf::from("libpam.so.0"),
        format!("/usr/lib/{architecture}-linux-gnu/libpam.so.0").into(),
        format!("/lib/{architecture}-linux-gnu/libpam.so.0").into(),
        std::path::PathBuf::from("/usr/lib/libpam.so.0"),
        std::path::PathBuf::from("/lib/libpam.so.0"),
        std::path::PathBuf::from("/usr/lib64/libpam.so.0"),
    ]);
    let mut last_error = None;
    for candidate in candidates {
        match unsafe { Library::new(&candidate) } {
            Ok(library) => return Ok(library),
            Err(error) => last_error = Some(error),
        }
    }
    Err(PamError::plain(format!(
        "could not load libpam: {}",
        last_error.expect("PAM library candidates are non-empty")
    )))
}

unsafe fn symbol<'library, T>(
    library: &'library Library,
    name: &[u8],
) -> Result<Symbol<'library, T>, PamError> {
    unsafe { library.get(name) }
        .map_err(|error| PamError::plain(format!("could not load PAM symbol: {error}")))
}

unsafe fn pam_error(handle: *mut PamHandle, code: c_int, strerror: &PamStrerror) -> PamError {
    let pointer = unsafe { strerror(handle, code) };
    let message = if pointer.is_null() {
        format!("PAM failed with code {code}")
    } else {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    };
    PamError {
        code: Some(code),
        message,
    }
}

#[cfg(test)]
mod tests;
