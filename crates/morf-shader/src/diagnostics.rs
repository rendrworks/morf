use std::fmt;

/// One compile error, pointing at a line of the shader the author wrote.
///
/// Diagnostics accumulate rather than stopping the compile: a configuration
/// author should see everything wrong with a shader in one run, not discover
/// the second mistake after fixing the first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// Line within the shader source, one-based.
    pub line: u32,
    /// What is wrong, in the terms the author used.
    pub message: String,
    /// How to say what they meant, when there is a way.
    pub note: Option<String>,
}

impl Diagnostic {
    pub fn new(line: u32, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
            note: None,
        }
    }

    /// Adds the line that tells the author what to write instead.
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.line, self.message)?;
        if let Some(note) = &self.note {
            write!(formatter, "\n  note: {note}")?;
        }
        Ok(())
    }
}

/// Renders a whole run's diagnostics, one per line, prefixed by the shader name.
pub fn report(name: &str, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| format!("{name}:{diagnostic}"))
        .collect::<Vec<_>>()
        .join("\n")
}
