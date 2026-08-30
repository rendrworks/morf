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

/// Incremental byte parser using an arbitrary delimiter.
pub struct SplitParser {
    delimiter: Vec<u8>,
    pending: Vec<u8>,
}

impl SplitParser {
    /// Creates a parser for the supplied delimiter.
    pub fn new(delimiter: impl Into<Vec<u8>>) -> io::Result<Self> {
        let delimiter = delimiter.into();
        Ok(Self {
            delimiter,
            pending: Vec::new(),
        })
    }

    /// Appends a chunk and returns every complete segment.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        if self.delimiter.is_empty() {
            let mut parts = Vec::new();
            if !self.pending.is_empty() {
                parts.push(std::mem::take(&mut self.pending));
            }
            if !chunk.is_empty() {
                parts.push(chunk.to_vec());
            }
            return parts;
        }
        self.pending.extend_from_slice(chunk);
        let mut parts = Vec::new();
        while let Some(at) = find_bytes(&self.pending, &self.delimiter) {
            parts.push(self.pending.drain(..at).collect());
            self.pending.drain(..self.delimiter.len());
        }
        parts
    }

    pub fn delimiter(&self) -> &[u8] {
        &self.delimiter
    }

    pub fn set_delimiter(&mut self, delimiter: impl Into<Vec<u8>>) -> Vec<Vec<u8>> {
        let delimiter = delimiter.into();
        if delimiter == self.delimiter {
            return Vec::new();
        }
        self.delimiter = delimiter;
        self.push(&[])
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

/// Bounded stream collector with optional end-of-stream publication.
pub struct StreamCollector {
    pending: Vec<u8>,
    data: Vec<u8>,
    maximum: usize,
    wait_for_end: bool,
    finished: bool,
}

impl StreamCollector {
    /// Creates a collector with an explicit byte limit.
    pub fn new(maximum: usize, wait_for_end: bool) -> io::Result<Self> {
        if maximum == 0 || maximum > 16 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream collector limit must be 1..16777216 bytes",
            ));
        }
        Ok(Self {
            pending: Vec::new(),
            data: Vec::new(),
            maximum,
            wait_for_end,
            finished: false,
        })
    }

    /// Appends bytes and returns whether the published value changed.
    pub fn push(&mut self, chunk: &[u8]) -> io::Result<bool> {
        if self.finished {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "stream collector is finished",
            ));
        }
        if self.pending.len().saturating_add(chunk.len()) > self.maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stream collector exceeded its byte limit",
            ));
        }
        self.pending.extend_from_slice(chunk);
        if self.wait_for_end {
            Ok(false)
        } else {
            self.data.clone_from(&self.pending);
            Ok(true)
        }
    }

    /// Publishes the final buffer and marks the stream finished.
    pub fn finish(&mut self) -> bool {
        if self.finished {
            return false;
        }
        self.finished = true;
        if self.wait_for_end {
            self.data.clone_from(&self.pending);
        }
        true
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.data).into_owned()
    }

    pub fn wait_for_end(&self) -> bool {
        self.wait_for_end
    }

    pub fn set_wait_for_end(&mut self, wait_for_end: bool) {
        if self.wait_for_end == wait_for_end {
            return;
        }
        self.wait_for_end = wait_for_end;
        if !wait_for_end {
            self.data.clone_from(&self.pending);
        }
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.data.clear();
        self.finished = false;
    }
}

