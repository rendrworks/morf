//! The conversation, as PAM means it: a module asks, somebody answers.
//!
//! Until this existed morf answered itself. The callback was handed a username
//! and a password up front and matched them to prompts by style — echo off
//! gets the password, echo on gets the username — and dropped everything else
//! on the floor. That is fine for a password and useless for anything a person
//! has to be part of: a fingerprint module says "touch the sensor" through
//! `PAM_TEXT_INFO` and nobody saw it; a hardware key asks a question the caller
//! never received; a wrong PIN's error message went nowhere.
//!
//! So the callback now hands every message to whoever started the transaction
//! and, for a prompt, waits for the answer. The same callback also serves the
//! old password-only case: the answers are simply supplied ahead of time rather
//! than one at a time. One conversation, two ways of answering it.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use crate::pam::{Credentials, PamError};

pub(crate) const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
pub(crate) const PAM_CONV_ERR: c_int = 19;

#[repr(C)]
pub(crate) struct PamMessage {
    style: c_int,
    message: *const c_char,
}

#[repr(C)]
pub(crate) struct PamResponse {
    response: *mut c_char,
    return_code: c_int,
}

/// One thing a PAM module said.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PamPrompt {
    /// A question that wants an answer. `echo` is the module's hint about
    /// whether the answer is secret — false for a password, true for a
    /// username or a one-time code the person can see on another device.
    Prompt { text: String, echo: bool },
    /// Something to show and move on from: "touch the sensor now".
    Info(String),
    /// Something went wrong, in the module's words: "wrong PIN, two tries left".
    Error(String),
}

/// What comes out of a running transaction.
#[derive(Debug)]
pub enum PamEvent {
    Message(PamPrompt),
    /// The transaction is over. Nothing follows this.
    Finished(Result<(), PamError>),
}

/// How prompts get answered.
pub(crate) enum Answers {
    /// One at a time, from whoever is watching the events.
    Interactive(mpsc::Receiver<String>),
    /// From credentials handed in up front — the password case. Prompts are
    /// answered without leaving this thread; informational messages still go
    /// out, because there is no reason to hide "your password expires
    /// tomorrow" from a caller that would have seen it interactively.
    Fixed(Credentials),
}

/// Shared between the transaction thread and the caller.
pub(crate) struct Bridge {
    pub(crate) events: mpsc::Sender<PamEvent>,
    pub(crate) answers: Answers,
    /// Set by `cancel`. Checked before every prompt, so a transaction that is
    /// asked to stop fails at the next question rather than the one after.
    pub(crate) cancelled: Arc<AtomicBool>,
}

impl Bridge {
    /// The answer to one prompt, or `None` when there will not be one.
    fn answer(&self, style: c_int) -> Option<Vec<u8>> {
        if self.cancelled.load(Ordering::SeqCst) {
            return None;
        }
        match &self.answers {
            Answers::Fixed(credentials) => Some(match style {
                PAM_PROMPT_ECHO_OFF => credentials.password.as_bytes().to_vec(),
                _ => credentials.username.as_bytes().to_vec(),
            }),
            // Blocks the transaction thread, which is the point: PAM is
            // synchronous and the person is not. A dropped sender -- the
            // session went away -- is a `None` here and a conversation error to
            // the module, which ends the transaction cleanly.
            Answers::Interactive(receiver) => receiver.recv().ok().map(String::into_bytes),
        }
    }
}

/// The callback PAM invokes, with `data` pointing at the [`Bridge`].
///
/// # Safety
/// Called by libpam with the documented argument shapes; `data` must be the
/// `*mut Bridge` registered with `pam_start`, alive for the whole transaction.
pub(crate) unsafe extern "C" fn conversation(
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
    let bridge = unsafe { &*(data.cast::<Bridge>()) };
    for index in 0..count {
        let message = unsafe { *messages.add(index) };
        if message.is_null() {
            unsafe { free_responses(allocated, count) };
            return PAM_CONV_ERR;
        }
        let style = unsafe { (*message).style };
        let text = unsafe { text_of((*message).message) };
        let answer = match style {
            PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => {
                let echo = style == PAM_PROMPT_ECHO_ON;
                // Told before being asked, so a display can show "PIN:" while
                // the thread waits. Delivery failing means nobody is listening,
                // and the answer below will be `None` for the same reason.
                let _ = bridge
                    .events
                    .send(PamEvent::Message(PamPrompt::Prompt { text, echo }));
                bridge.answer(style)
            }
            PAM_TEXT_INFO => {
                let _ = bridge.events.send(PamEvent::Message(PamPrompt::Info(text)));
                None
            }
            PAM_ERROR_MSG => {
                let _ = bridge
                    .events
                    .send(PamEvent::Message(PamPrompt::Error(text)));
                None
            }
            _ => {
                unsafe { free_responses(allocated, count) };
                return PAM_CONV_ERR;
            }
        };
        let is_prompt = matches!(style, PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON);
        match answer {
            Some(bytes) => {
                let Some(response) = (unsafe { duplicate(&bytes) }) else {
                    unsafe { free_responses(allocated, count) };
                    return PAM_CONV_ERR;
                };
                unsafe { (*allocated.add(index)).response = response };
            }
            // A prompt with no answer is the transaction being cancelled or
            // abandoned. Anything else with no answer is a message that never
            // wanted one.
            None if is_prompt => {
                unsafe { free_responses(allocated, count) };
                return PAM_CONV_ERR;
            }
            None => {}
        }
    }
    unsafe { responses.write(allocated) };
    PAM_SUCCESS
}

/// The message text, or empty when the module sent none.
unsafe fn text_of(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

/// A malloc'd, NUL-terminated copy, which is what PAM frees.
///
/// Not `strdup`: an answer may legitimately contain a byte the caller typed
/// that is not valid in a C string, and refusing it here is better than
/// truncating a password at the first zero.
unsafe fn duplicate(bytes: &[u8]) -> Option<*mut c_char> {
    if bytes.contains(&0) {
        return None;
    }
    let copy = unsafe { libc::malloc(bytes.len() + 1) }.cast::<u8>();
    if copy.is_null() {
        return None;
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), copy, bytes.len());
        copy.add(bytes.len()).write(0);
    }
    Some(copy.cast())
}

pub(crate) unsafe fn free_responses(responses: *mut PamResponse, count: usize) {
    for index in 0..count {
        unsafe { libc::free((*responses.add(index)).response.cast()) };
    }
    unsafe { libc::free(responses.cast()) };
}
