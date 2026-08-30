/// Connected Unix-domain byte stream.
pub struct Socket(UnixStream);

impl Socket {
    pub fn pair() -> io::Result<(Self, Self)> {
        UnixStream::pair().map(|(left, right)| (Self(left), Self(right)))
    }

    pub fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        UnixStream::connect(path).map(Self)
    }

    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.0.shutdown(Shutdown::Both)
    }

    pub fn receive(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }

    /// Receives bytes with a temporary read timeout.
    pub fn receive_timeout(&mut self, bytes: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.0.set_read_timeout(Some(timeout))?;
        let result = self.0.read(bytes);
        let _ = self.0.set_read_timeout(None);
        result
    }
}

/// Listening Unix-domain socket.
pub struct SocketServer {
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}

impl SocketServer {
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let listener = UnixListener::bind(path)?;
        let metadata = fs::metadata(path)?;
        Ok(Self {
            listener,
            path: path.to_owned(),
            identity: (metadata.dev(), metadata.ino()),
        })
    }

    pub fn accept(&self) -> io::Result<Socket> {
        self.listener.accept().map(|(stream, _)| Socket(stream))
    }

    /// Accepts one pending client without blocking the caller.
    pub fn try_accept(&self) -> io::Result<Option<Socket>> {
        self.listener.set_nonblocking(true)?;
        let result = match self.listener.accept() {
            Ok((stream, _)) => Ok(Some(Socket(stream))),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        };
        let reset = self.listener.set_nonblocking(false);
        match (result, reset) {
            (Ok(socket), Ok(())) => Ok(socket),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        if fs::metadata(&self.path)
            .map(|metadata| (metadata.dev(), metadata.ino()) == self.identity)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

