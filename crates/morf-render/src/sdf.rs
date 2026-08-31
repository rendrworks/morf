use morf_scene::SceneError;
use std::error::Error as StdError;
use std::fmt;

/// Scene or backend failure while producing a frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// Scene property or handle failure.
    Scene(String),
    /// Selected rendering backend failure.
    Backend(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(message) => write!(f, "scene paint error: {message}"),
            Self::Backend(message) => write!(f, "render backend error: {message}"),
        }
    }
}

impl StdError for RenderError {}

impl From<SceneError> for RenderError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error.to_string())
    }
}
