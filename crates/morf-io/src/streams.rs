use std::io;

/// Incremental byte parser using an arbitrary delimiter.
pub struct SplitParser {
    delimiter: Vec<u8>,
    pending: Vec<u8>,
}

impl SplitParser {
    /// Creates a parser for the supplied delimiter.
    ///
    /// An empty delimiter is not an error: it means "do not frame at all", and
    /// `push` passes chunks through as they arrive. There is nothing else here
    /// that can fail, so this returns a parser rather than a `Result` that
    /// could only ever be `Ok` — which read, at the call site, as though a
    /// check existed somewhere.
    pub fn new(delimiter: impl Into<Vec<u8>>) -> Self {
        Self {
            delimiter: delimiter.into(),
            pending: Vec::new(),
        }
    }

    /// Whether a segment ended by this delimiter should lose a trailing `\r`.
    ///
    /// Only for the newline delimiter, and only because a line ending is two
    /// characters on one of the two systems that write text files. A parser
    /// splitting on `--` has no such convention and must not invent one.
    fn trims_carriage_return(&self) -> bool {
        self.delimiter == b"\n"
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
        let trim = self.trims_carriage_return();
        let mut parts = Vec::new();
        while let Some(at) = find_bytes(&self.pending, &self.delimiter) {
            let mut part: Vec<u8> = self.pending.drain(..at).collect();
            if trim && part.last() == Some(&b'\r') {
                part.pop();
            }
            parts.push(part);
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
        Ok(!self.wait_for_end)
    }

    /// Publishes the final buffer and marks the stream finished.
    pub fn finish(&mut self) -> bool {
        if self.finished {
            return false;
        }
        self.finished = true;
        true
    }

    /// What has been collected so far, or nothing until the stream ends.
    ///
    /// This used to be a second copy of the buffer, refreshed on every push —
    /// so collecting a stream of `n` bytes in 8 KiB chunks copied `n²/16384`
    /// bytes and held twice the memory. What a caller waiting for the end must
    /// not see is a partial buffer, and that is a question about `finished`,
    /// not a reason to keep the bytes twice.
    pub fn data(&self) -> &[u8] {
        if self.wait_for_end && !self.finished {
            return &[];
        }
        &self.pending
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(self.data()).into_owned()
    }

    pub fn wait_for_end(&self) -> bool {
        self.wait_for_end
    }

    pub fn set_wait_for_end(&mut self, wait_for_end: bool) {
        if self.wait_for_end == wait_for_end {
            return;
        }
        self.wait_for_end = wait_for_end;
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.finished = false;
    }
}
