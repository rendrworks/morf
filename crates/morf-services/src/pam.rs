use std::error::Error as StdError;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::mem::size_of;
use std::ptr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use libloading::{Library, Symbol};

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_CONV_ERR: c_int = 19;
const PAM_DISALLOW_NULL_AUTHTOK: c_int = 1;

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct PamMessage {
    style: c_int,
    message: *const c_char,
}

#[repr(C)]
struct PamResponse {
    response: *mut c_char,
    return_code: c_int,
}

#[repr(C)]
struct PamConversation {
    callback: Option<
        unsafe extern "C" fn(
            c_int,
            *const *const PamMessage,
            *mut *mut PamResponse,
            *mut c_void,
        ) -> c_int,
    >,
    data: *mut c_void,
}

struct Credentials {
    username: CString,
    password: CString,
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
}

impl fmt::Display for PamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for PamError {}

/// Synchronous Linux-PAM authentication and account validation.
pub struct PamAuthenticator;

/// Authentication result produced away from the shell event loop.
pub struct PamTask {
    result: mpsc::Receiver<Result<(), PamError>>,
}

impl PamTask {
    /// Waits up to the supplied duration for PAM to finish.
    pub fn wait(&self, timeout: Duration) -> Option<Result<(), PamError>> {
        self.result.recv_timeout(timeout).ok()
    }
}

impl PamAuthenticator {
    /// Authenticates credentials through the named PAM service.
    pub fn authenticate(service: &str, username: &str, password: &str) -> Result<(), PamError> {
        let service = cstring("service", service)?;
        let mut credentials = Credentials {
            username: cstring("username", username)?,
            password: cstring("password", password)?,
        };
        let library = load_pam()?;
        unsafe { authenticate(&library, &service, &mut credentials) }
    }

    /// Starts authentication on a dedicated worker thread.
    pub fn authenticate_async(
        service: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> PamTask {
        let service = service.into();
        let username = username.into();
        let mut password = password.into();
        let (tx, result) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let outcome = Self::authenticate(&service, &username, &password);
            for byte in unsafe { password.as_bytes_mut() } {
                unsafe { ptr::from_mut(byte).write_volatile(0) };
            }
            let _ = tx.send(outcome);
        });
        PamTask { result }
    }
}

fn cstring(field: &str, value: &str) -> Result<CString, PamError> {
    CString::new(value).map_err(|_| PamError {
        code: None,
        message: format!("{field} contains a null byte"),
    })
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
    Err(PamError {
        code: None,
        message: format!(
            "could not load libpam: {}",
            last_error.expect("PAM library candidates are non-empty")
        ),
    })
}

unsafe fn authenticate(
    library: &Library,
    service: &CStr,
    credentials: &mut Credentials,
) -> Result<(), PamError> {
    let start: Symbol<'_, PamStart> = unsafe { symbol(library, b"pam_start\0")? };
    let authenticate: Symbol<'_, PamAuthenticate> =
        unsafe { symbol(library, b"pam_authenticate\0")? };
    let account: Symbol<'_, PamAccount> = unsafe { symbol(library, b"pam_acct_mgmt\0")? };
    let end: Symbol<'_, PamEnd> = unsafe { symbol(library, b"pam_end\0")? };
    let strerror: Symbol<'_, PamStrerror> = unsafe { symbol(library, b"pam_strerror\0")? };
    let conversation = PamConversation {
        callback: Some(conversation),
        data: ptr::from_mut(credentials).cast(),
    };
    let mut handle = ptr::null_mut();
    let mut status = unsafe {
        start(
            service.as_ptr(),
            credentials.username.as_ptr(),
            &conversation,
            &mut handle,
        )
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
    } else {
        Err(unsafe { pam_error(handle, status, &strerror) })
    };
    let end_status = unsafe { end(handle, status) };
    if result.is_ok() && end_status != PAM_SUCCESS {
        return Err(unsafe { pam_error(ptr::null_mut(), end_status, &strerror) });
    }
    result
}

unsafe fn symbol<'library, T>(
    library: &'library Library,
    name: &[u8],
) -> Result<Symbol<'library, T>, PamError> {
    unsafe { library.get(name) }.map_err(|error| PamError {
        code: None,
        message: format!("could not load PAM symbol: {error}"),
    })
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

unsafe extern "C" fn conversation(
    count: c_int,
    messages: *const *const PamMessage,
    responses: *mut *mut PamResponse,
    data: *mut c_void,
) -> c_int {
    if count <= 0 || messages.is_null() || responses.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    unsafe { responses.write(ptr::null_mut()) };
    let count = count as usize;
    let allocated = unsafe { libc::calloc(count, size_of::<PamResponse>()) }.cast::<PamResponse>();
    if allocated.is_null() {
        return PAM_CONV_ERR;
    }
    let credentials = unsafe { &*(data.cast::<Credentials>()) };
    for index in 0..count {
        let message = unsafe { *messages.add(index) };
        if message.is_null() {
            unsafe { free_responses(allocated, count) };
            return PAM_CONV_ERR;
        }
        let source = match unsafe { (*message).style } {
            PAM_PROMPT_ECHO_OFF => Some(credentials.password.as_ptr()),
            PAM_PROMPT_ECHO_ON => Some(credentials.username.as_ptr()),
            PAM_ERROR_MSG | PAM_TEXT_INFO => None,
            _ => {
                unsafe { free_responses(allocated, count) };
                return PAM_CONV_ERR;
            }
        };
        if let Some(source) = source {
            let response = unsafe { libc::strdup(source) };
            if response.is_null() {
                unsafe { free_responses(allocated, count) };
                return PAM_CONV_ERR;
            }
            unsafe { (*allocated.add(index)).response = response };
        }
    }
    unsafe { responses.write(allocated) };
    PAM_SUCCESS
}

unsafe fn free_responses(responses: *mut PamResponse, count: usize) {
    for index in 0..count {
        unsafe { libc::free((*responses.add(index)).response.cast()) };
    }
    unsafe { libc::free(responses.cast()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_embedded_nulls_before_starting_pam() {
        let error = PamAuthenticator::authenticate("morf\0test", "user", "secret").unwrap_err();
        assert_eq!(error.code(), None);
        assert_eq!(error.to_string(), "service contains a null byte");
    }

    #[test]
    fn asynchronous_authentication_returns_without_blocking_caller() {
        let task = PamAuthenticator::authenticate_async("morf\0test", "user", "secret");
        let error = task
            .wait(Duration::from_secs(1))
            .expect("PAM worker returned")
            .unwrap_err();
        assert_eq!(error.to_string(), "service contains a null byte");
    }
}
