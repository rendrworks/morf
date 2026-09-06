//! The loop's alarm: a way for a thread with news to end a poll early.
//!
//! Every service here runs on its own thread and reports through a channel,
//! and the loop that drains those channels sleeps in a `poll` on its Wayland
//! socket. Without this it woke only for the compositor or for its own
//! fallback timeout, so a timer, a line from a child, or a signal from the bus
//! waited up to that timeout to be seen — the whole shell felt a tenth of a
//! second behind. A `Wake` is an eventfd the loop polls alongside the socket;
//! `wake_all` pokes every one there is, so a thread need not know which loop
//! is waiting for it. The poke is cheap and idempotent, and a loop that is
//! busy rather than waiting simply finds the fd readable next time round.

use rustix::event::{EventfdFlags, eventfd};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::io::{read, write};
use std::io;
use std::sync::{Arc, Mutex};

static WAKES: Mutex<Vec<Arc<OwnedFd>>> = Mutex::new(Vec::new());

/// One loop's alarm; registered for the life of the value.
pub struct Wake {
    fd: Arc<OwnedFd>,
}

impl Wake {
    pub fn new() -> io::Result<Self> {
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        let fd = Arc::new(fd);
        WAKES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(Arc::clone(&fd));
        Ok(Self { fd })
    }

    /// Clears the alarm; called after the poll returned because of it.
    pub fn drain(&self) {
        let mut buffer = [0u8; 8];
        let _ = read(&self.fd, &mut buffer);
    }
}

impl AsFd for Wake {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl Drop for Wake {
    fn drop(&mut self) {
        WAKES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|other| !Arc::ptr_eq(other, &self.fd));
    }
}

/// Tells every waiting loop there is something to collect.
pub fn wake_all() {
    let wakes = WAKES.lock().unwrap_or_else(|error| error.into_inner());
    for fd in wakes.iter() {
        let _ = write(fd, &1u64.to_ne_bytes());
    }
}
