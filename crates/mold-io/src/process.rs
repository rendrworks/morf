/// One event emitted by a child process.
#[derive(Debug)]
pub enum ProcessEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(ExitStatus),
}

/// Spawned child with streamed output and writable stdin.
pub struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    events: mpsc::Receiver<ProcessEvent>,
    exit_reported: bool,
}

/// Native process launch settings without shell interpretation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessConfig {
    pub command: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub clear_environment: bool,
    pub working_directory: Option<PathBuf>,
}

impl Process {
    /// Spawns a child without invoking a shell.
    pub fn spawn<I, S>(program: impl AsRef<std::ffi::OsStr>, args: I) -> io::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(program);
        command.args(args);
        Self::spawn_command(&mut command)
    }

    pub fn spawn_config(config: &ProcessConfig) -> io::Result<Self> {
        let (program, args) = config.command.split_first().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "process command cannot be empty",
            )
        })?;
        let mut command = Command::new(program);
        command.args(args);
        if config.clear_environment {
            command.env_clear();
        }
        command.envs(&config.environment);
        if let Some(directory) = &config.working_directory {
            command.current_dir(directory);
        }
        Self::spawn_command(&mut command)
    }

    fn spawn_command(command: &mut Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout is present");
        let stderr = child.stderr.take().expect("piped stderr is present");
        let (tx, events) = mpsc::channel();
        stream_reader(stdout, tx.clone(), ProcessEvent::Stdout);
        stream_reader(stderr, tx, ProcessEvent::Stderr);
        Ok(Self {
            child,
            stdin,
            events,
            exit_reported: false,
        })
    }

    /// Writes bytes to the child's standard input.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "child stdin is closed"))?
            .write_all(bytes)
    }

    /// Closes the child's standard input.
    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn signal(&self, signal: i32) -> io::Result<()> {
        if !(1..=64).contains(&signal) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "signal must be 1..64",
            ));
        }
        let result = unsafe { libc::kill(self.child.id() as i32, signal) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Waits up to the supplied duration for output or process exit.
    pub fn next_event(&mut self, timeout: Duration) -> io::Result<Option<ProcessEvent>> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => return Ok(Some(event)),
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }
        if !self.exit_reported
            && let Some(status) = self.child.try_wait()?
        {
            self.exit_reported = true;
            return Ok(Some(ProcessEvent::Exit(status)));
        }
        Ok(None)
    }

    /// Requests child termination.
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stdin = None;
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn stream_reader<R, F>(mut reader: R, tx: mpsc::Sender<ProcessEvent>, event: F)
where
    R: Read + Send + 'static,
    F: Fn(Vec<u8>) -> ProcessEvent + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = vec![0; 8 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) if tx.send(event(buffer[..read].to_vec())).is_err() => break,
                Ok(_) => {}
            }
        }
    });
}

