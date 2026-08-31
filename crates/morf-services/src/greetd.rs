use std::env;
use std::error::Error as StdError;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

const MAX_MESSAGE_SIZE: usize = 1024 * 1024;

/// Authentication prompt classification supplied by greetd.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthMessageType {
    Visible,
    Secret,
    Info,
    Error,
}

/// One response received from greetd.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GreetdResponse {
    Success,
    AuthMessage {
        kind: AuthMessageType,
        message: String,
    },
    Error {
        authentication: bool,
        description: String,
    },
}

/// greetd transport or protocol failure.
#[derive(Debug)]
pub enum GreetdError {
    Io(io::Error),
    Protocol(String),
}

impl fmt::Display for GreetdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "greetd I/O error: {error}"),
            Self::Protocol(message) => write!(formatter, "greetd protocol error: {message}"),
        }
    }
}

impl StdError for GreetdError {}

impl From<io::Error> for GreetdError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Length-prefixed JSON client for one greetd connection.
pub struct GreetdClient {
    stream: UnixStream,
}

impl GreetdClient {
    /// Connects to a greetd Unix socket with bounded blocking operations.
    pub fn connect(path: impl AsRef<Path>, timeout: Duration) -> Result<Self, GreetdError> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(Self { stream })
    }

    /// Connects to the socket named by `GREETD_SOCK`.
    pub fn connect_environment(timeout: Duration) -> Result<Self, GreetdError> {
        let path = env::var_os("GREETD_SOCK")
            .ok_or_else(|| GreetdError::Protocol("GREETD_SOCK is unset".to_owned()))?;
        Self::connect(path, timeout)
    }

    /// Starts authentication for a username.
    pub fn create_session(&mut self, username: &str) -> Result<GreetdResponse, GreetdError> {
        self.request(json!({ "type": "create_session", "username": username }))
    }

    /// Answers the current authentication message.
    pub fn respond(&mut self, response: Option<&str>) -> Result<GreetdResponse, GreetdError> {
        self.request(json!({
            "type": "post_auth_message_response",
            "response": response,
        }))
    }

    /// Starts an authenticated session command.
    pub fn start_session(
        &mut self,
        command: &[String],
        environment: &[String],
    ) -> Result<GreetdResponse, GreetdError> {
        self.request(json!({
            "type": "start_session",
            "cmd": command,
            "env": environment,
        }))
    }

    /// Cancels the current login flow.
    pub fn cancel_session(&mut self) -> Result<GreetdResponse, GreetdError> {
        self.request(json!({ "type": "cancel_session" }))
    }

    fn request(&mut self, request: Value) -> Result<GreetdResponse, GreetdError> {
        let payload = serde_json::to_vec(&request)
            .map_err(|error| GreetdError::Protocol(error.to_string()))?;
        if payload.len() > MAX_MESSAGE_SIZE {
            return Err(GreetdError::Protocol("request is too large".to_owned()));
        }
        let length = u32::try_from(payload.len())
            .map_err(|_| GreetdError::Protocol("request is too large".to_owned()))?;
        self.stream.write_all(&length.to_ne_bytes())?;
        self.stream.write_all(&payload)?;
        self.stream.flush()?;

        let mut length = [0_u8; 4];
        self.stream.read_exact(&mut length)?;
        let length = u32::from_ne_bytes(length) as usize;
        if length > MAX_MESSAGE_SIZE {
            return Err(GreetdError::Protocol("response is too large".to_owned()));
        }
        let mut payload = vec![0_u8; length];
        self.stream.read_exact(&mut payload)?;
        let response: Value = serde_json::from_slice(&payload)
            .map_err(|error| GreetdError::Protocol(error.to_string()))?;
        parse_response(&response)
    }
}

fn parse_response(response: &Value) -> Result<GreetdResponse, GreetdError> {
    let object = response
        .as_object()
        .ok_or_else(|| GreetdError::Protocol("response must be an object".to_owned()))?;
    match object.get("type").and_then(Value::as_str) {
        Some("success") => Ok(GreetdResponse::Success),
        Some("auth_message") => {
            let kind = match object.get("auth_message_type").and_then(Value::as_str) {
                Some("visible") => AuthMessageType::Visible,
                Some("secret") => AuthMessageType::Secret,
                Some("info") => AuthMessageType::Info,
                Some("error") => AuthMessageType::Error,
                _ => {
                    return Err(GreetdError::Protocol(
                        "invalid authentication message type".to_owned(),
                    ));
                }
            };
            let message = object
                .get("auth_message")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    GreetdError::Protocol("authentication message is missing".to_owned())
                })?;
            Ok(GreetdResponse::AuthMessage {
                kind,
                message: message.to_owned(),
            })
        }
        Some("error") => {
            let error_type = object
                .get("error_type")
                .and_then(Value::as_str)
                .ok_or_else(|| GreetdError::Protocol("error type is missing".to_owned()))?;
            let description = object
                .get("description")
                .and_then(Value::as_str)
                .ok_or_else(|| GreetdError::Protocol("error description is missing".to_owned()))?;
            match error_type {
                "error" | "auth_error" => Ok(GreetdResponse::Error {
                    authentication: error_type == "auth_error",
                    description: description.to_owned(),
                }),
                _ => Err(GreetdError::Protocol("invalid error type".to_owned())),
            }
        }
        _ => Err(GreetdError::Protocol("invalid response type".to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn client_exchanges_native_length_prefixed_messages() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let request = read_value(&mut server);
            assert_eq!(request["type"], "create_session");
            assert_eq!(request["username"], "morf");
            write_value(
                &mut server,
                &json!({
                    "type": "auth_message",
                    "auth_message_type": "secret",
                    "auth_message": "Password:",
                }),
            );
        });
        let mut client = GreetdClient { stream: client };

        assert_eq!(
            client.create_session("morf").unwrap(),
            GreetdResponse::AuthMessage {
                kind: AuthMessageType::Secret,
                message: "Password:".to_owned(),
            }
        );
        server.join().unwrap();
    }

    #[test]
    fn response_parser_distinguishes_authentication_errors() {
        assert_eq!(
            parse_response(&json!({
                "type": "error",
                "error_type": "auth_error",
                "description": "invalid credentials",
            }))
            .unwrap(),
            GreetdResponse::Error {
                authentication: true,
                description: "invalid credentials".to_owned(),
            }
        );
    }

    fn read_value(stream: &mut UnixStream) -> Value {
        let mut length = [0_u8; 4];
        stream.read_exact(&mut length).unwrap();
        let mut payload = vec![0_u8; u32::from_ne_bytes(length) as usize];
        stream.read_exact(&mut payload).unwrap();
        serde_json::from_slice(&payload).unwrap()
    }

    fn write_value(stream: &mut UnixStream, value: &Value) {
        let payload = serde_json::to_vec(value).unwrap();
        stream
            .write_all(&(payload.len() as u32).to_ne_bytes())
            .unwrap();
        stream.write_all(&payload).unwrap();
    }
}
