//! A greetd login as a conversation, off the thread that draws.
//!
//! [`GreetdClient`] is one request and one reply, and the reply comes when
//! greetd's PAM stack has something to say. Between a question and the next
//! there may be a module waiting on a person — a fingerprint reader with the
//! prompt "place your finger" already delivered — and that wait is greetd's,
//! not something the client can shorten. A greeter that made these calls on
//! its drawing thread stood still for the whole of it.
//!
//! So the socket lives on a thread of its own here. What greetd says comes
//! out as [`GreetdEvent`]s to be polled; what the greeter answers goes in as
//! commands. The thread makes one request at a time, in the order the
//! protocol allows, and ends when the session is started, cancelled, or lost.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use crate::greetd::{GreetdClient, GreetdError, GreetdResponse};

/// How long one reply may take. A PAM module waiting on a person is the
/// case, and a person is slow: this is not a timeout on the person but on a
/// greetd that has stopped answering altogether.
const PATIENCE: Duration = Duration::from_secs(600);

/// Something greetd said, or the fact that it can no longer be asked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GreetdEvent {
    /// A reply to the last request: a question, a success, or a refusal.
    Response(GreetdResponse),
    /// The connection is gone, in the transport's words. Nothing follows.
    Failed(String),
}

enum Command {
    Respond(Option<String>),
    Start(Vec<String>, Vec<String>),
    Cancel,
}

/// One login, from `create_session` to a started or cancelled session.
pub struct GreetdConversation {
    events: Receiver<GreetdEvent>,
    commands: Option<Sender<Command>>,
    ended: bool,
}

impl GreetdConversation {
    /// Connects and asks for a session, on a thread of its own.
    ///
    /// `path` names the socket; `None` reads `GREETD_SOCK`, which is what
    /// greetd itself provides. The first event is greetd's answer to the
    /// session request — usually its first question.
    pub fn begin(path: Option<PathBuf>, username: String) -> Self {
        let (event_sender, events) = mpsc::channel();
        let (command_sender, commands) = mpsc::channel::<Command>();
        thread::spawn(move || {
            let connected = match path {
                Some(path) => GreetdClient::connect(path, PATIENCE),
                None => GreetdClient::connect_environment(PATIENCE),
            };
            let mut client = match connected {
                Ok(client) => client,
                Err(error) => {
                    let _ = event_sender.send(GreetdEvent::Failed(error.to_string()));
                    return;
                }
            };
            let deliver = |result: Result<GreetdResponse, GreetdError>| match result {
                Ok(response) => event_sender.send(GreetdEvent::Response(response)).is_ok(),
                Err(error) => {
                    let _ = event_sender.send(GreetdEvent::Failed(error.to_string()));
                    false
                }
            };
            if !deliver(client.create_session(&username)) {
                return;
            }
            // Ends when the other side drops its sender: the conversation
            // was let go of, and there is nobody left to hear an answer.
            for command in commands {
                let (result, last) = match command {
                    Command::Respond(answer) => (client.respond(answer.as_deref()), false),
                    Command::Start(command, environment) => {
                        (client.start_session(&command, &environment), true)
                    }
                    Command::Cancel => (client.cancel_session(), true),
                };
                if !deliver(result) || last {
                    return;
                }
            }
        });
        Self {
            events,
            commands: Some(command_sender),
            ended: false,
        }
    }

    /// The next thing greetd said, if it has said anything.
    ///
    /// `Duration::ZERO` is a poll. Once the thread is gone this returns
    /// `None` for good, and [`Self::ended`] says so.
    pub fn next(&mut self, timeout: Duration) -> Option<GreetdEvent> {
        if self.ended {
            return None;
        }
        let received = if timeout.is_zero() {
            match self.events.try_recv() {
                Ok(event) => Some(event),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => None,
            }
        } else {
            self.events.recv_timeout(timeout).ok()
        };
        if received.is_none() {
            self.ended = true;
        }
        received
    }

    /// Whether the thread has finished: the session was started, cancelled,
    /// or the connection was lost, and every event has been read.
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// Answers the last question. `None` is the answer to a message that
    /// asked nothing — an `info` or an `error` — which greetd still wants
    /// acknowledged before it goes on.
    pub fn respond(&self, answer: Option<String>) -> bool {
        self.send(Command::Respond(answer))
    }

    /// Asks greetd to start the session, after it has said `success`.
    pub fn start_session(&mut self, command: Vec<String>, environment: Vec<String>) -> bool {
        let sent = self.send(Command::Start(command, environment));
        self.commands = None;
        sent
    }

    /// Gives the login up. greetd answers this once its current module has
    /// returned; a reader mid-wait cannot be interrupted from here.
    pub fn cancel(&mut self) -> bool {
        let sent = self.send(Command::Cancel);
        self.commands = None;
        sent
    }

    fn send(&self, command: Command) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.send(command).is_ok())
    }
}
