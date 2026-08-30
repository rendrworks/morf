const IPC_MAX_CONNECTIONS: usize = 32;
const IPC_MAX_REQUEST: usize = 64 * 1024;
const IPC_MAX_RESPONSE: usize = 256 * 1024;
const IPC_TIMEOUT: Duration = Duration::from_secs(2);

/// Primitive value carried by the public IPC protocol.
#[derive(Clone, Debug, PartialEq)]
pub enum IpcValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
}

/// One decoded IPC operation.
#[derive(Clone, Debug, PartialEq)]
pub enum IpcRequest {
    Call { target: String, args: Vec<IpcValue> },
    Verbs,
    Log,
    Bindings,
    Kill,
}

/// Credentials read from the accepted Unix socket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

/// Reply returned to one IPC request.
#[derive(Clone, Debug, PartialEq)]
pub struct IpcReply {
    pub ok: bool,
    pub result: Vec<IpcValue>,
    pub error: Option<String>,
}

impl IpcReply {
    pub fn success(result: Vec<IpcValue>) -> Self {
        Self {
            ok: true,
            result,
            error: None,
        }
    }

    pub fn refused(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            result: Vec::new(),
            error: Some(error.into()),
        }
    }

    pub fn to_wire(&self) -> io::Result<Vec<u8>> {
        encode_ipc_reply(self)
    }
}

/// Request handed from socket workers to the shell event loop.
pub struct IpcIncoming {
    pub peer: PeerCredentials,
    pub request: IpcRequest,
    reply: mpsc::SyncSender<IpcReply>,
}

impl IpcIncoming {
    pub fn reply(self, reply: IpcReply) {
        let _ = self.reply.send(reply);
    }
}

/// Bounded persistent Unix-socket IPC server.
pub struct IpcServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn bind(path: impl Into<PathBuf>, requests: mpsc::Sender<IpcIncoming>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() {
            match UnixStream::connect(&path) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "IPC socket is already active",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    fs::remove_file(&path)?;
                }
                Err(error) => return Err(error),
            }
        }
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let active = Arc::new(AtomicUsize::new(0));
        let join = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if active.fetch_add(1, Ordering::AcqRel) >= IPC_MAX_CONNECTIONS {
                            active.fetch_sub(1, Ordering::AcqRel);
                            let _ =
                                write_ipc_reply(stream, IpcReply::refused("too many connections"));
                            continue;
                        }
                        let requests = requests.clone();
                        let active = Arc::clone(&active);
                        thread::spawn(move || {
                            serve_ipc_connection(stream, requests);
                            active.fetch_sub(1, Ordering::AcqRel);
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            path,
            stop,
            join: Some(join),
        })
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// Sends one request and reads one wire reply.
pub fn ipc_call(path: impl AsRef<Path>, request: &IpcRequest) -> io::Result<IpcReply> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(IPC_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_TIMEOUT))?;
    let mut request = encode_ipc_request(request)?;
    request.push(b'\n');
    stream.write_all(&request)?;
    let mut response = Vec::new();
    BufReader::new(stream)
        .take(IPC_MAX_RESPONSE as u64 + 1)
        .read_until(b'\n', &mut response)?;
    if response.len() > IPC_MAX_RESPONSE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC response exceeds size limit",
        ));
    }
    decode_ipc_reply(&response)
}

fn serve_ipc_connection(mut stream: UnixStream, requests: mpsc::Sender<IpcIncoming>) {
    if stream.set_read_timeout(Some(IPC_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(IPC_TIMEOUT)).is_err()
    {
        return;
    }
    let Ok(peer) = peer_credentials(&stream) else {
        return;
    };
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream);
    loop {
        let mut line = Vec::new();
        match reader
            .by_ref()
            .take(IPC_MAX_REQUEST as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => break,
            Ok(_) if line.len() > IPC_MAX_REQUEST => {
                let _ =
                    write_ipc_reply(&mut stream, IpcReply::refused("request exceeds size limit"));
                break;
            }
            Ok(_) => {}
        }
        let request = match decode_ipc_request(&line) {
            Ok(request) => request,
            Err(error) => {
                let _ = write_ipc_reply(&mut stream, IpcReply::refused(error.to_string()));
                continue;
            }
        };
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if requests
            .send(IpcIncoming {
                peer,
                request,
                reply: reply_tx,
            })
            .is_err()
        {
            break;
        }
        let reply = reply_rx
            .recv_timeout(IPC_TIMEOUT)
            .unwrap_or_else(|_| IpcReply::refused("request timed out"));
        if write_ipc_reply(&mut stream, reply).is_err() {
            break;
        }
    }
}

fn peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(credentials).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCredentials {
        pid: credentials.pid as u32,
        uid: credentials.uid,
        gid: credentials.gid,
    })
}

fn write_ipc_reply(mut stream: impl Write, reply: IpcReply) -> io::Result<()> {
    let mut bytes = encode_ipc_reply(&reply)?;
    if bytes.len() > IPC_MAX_RESPONSE {
        bytes = encode_ipc_reply(&IpcReply::refused("response exceeds size limit"))?;
    }
    bytes.push(b'\n');
    stream.write_all(&bytes)
}

fn encode_ipc_request(request: &IpcRequest) -> io::Result<Vec<u8>> {
    let value = match request {
        IpcRequest::Call { target, args } => serde_json::json!({
            "op": "call",
            "target": target,
            "args": args.iter().map(ipc_value_to_json).collect::<Vec<_>>(),
        }),
        IpcRequest::Verbs => serde_json::json!({ "op": "verbs" }),
        IpcRequest::Log => serde_json::json!({ "op": "log" }),
        IpcRequest::Bindings => serde_json::json!({ "op": "bindings" }),
        IpcRequest::Kill => serde_json::json!({ "op": "kill" }),
    };
    serde_json::to_vec(&value).map_err(io::Error::other)
}

fn decode_ipc_request(bytes: &[u8]) -> io::Result<IpcRequest> {
    let value: JsonValue = serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let object = value
        .as_object()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "request must be an object"))?;
    let op = object
        .get("op")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "request needs string op"))?;
    match op {
        "call" => {
            let target = object
                .get("target")
                .and_then(JsonValue::as_str)
                .filter(|target| !target.is_empty() && target.len() <= 256)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid call target"))?;
            let args = object
                .get("args")
                .and_then(JsonValue::as_array)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "call args must be an array")
                })?;
            if args.len() > 64 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "too many call args",
                ));
            }
            Ok(IpcRequest::Call {
                target: target.to_owned(),
                args: args
                    .iter()
                    .map(ipc_value_from_json)
                    .collect::<io::Result<_>>()?,
            })
        }
        "verbs" => Ok(IpcRequest::Verbs),
        "log" => Ok(IpcRequest::Log),
        "bindings" => Ok(IpcRequest::Bindings),
        "kill" => Ok(IpcRequest::Kill),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown IPC operation",
        )),
    }
}

fn encode_ipc_reply(reply: &IpcReply) -> io::Result<Vec<u8>> {
    let mut object = JsonMap::new();
    object.insert("ok".into(), JsonValue::Bool(reply.ok));
    object.insert("n".into(), JsonValue::from(reply.result.len()));
    object.insert(
        "result".into(),
        JsonValue::Array(reply.result.iter().map(ipc_value_to_json).collect()),
    );
    if let Some(error) = &reply.error {
        object.insert("error".into(), JsonValue::String(error.clone()));
    }
    serde_json::to_vec(&JsonValue::Object(object)).map_err(io::Error::other)
}

fn decode_ipc_reply(bytes: &[u8]) -> io::Result<IpcReply> {
    let value: JsonValue = serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let object = value
        .as_object()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "reply must be an object"))?;
    let ok = object
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "reply needs boolean ok"))?;
    let result = object
        .get("result")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "reply result must be an array"))?
        .iter()
        .map(ipc_value_from_json)
        .collect::<io::Result<Vec<_>>>()?;
    let error = object
        .get("error")
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "reply error must be a string")
            })
        })
        .transpose()?;
    Ok(IpcReply { ok, result, error })
}

fn ipc_value_to_json(value: &IpcValue) -> JsonValue {
    match value {
        IpcValue::Nil => JsonValue::Null,
        IpcValue::Boolean(value) => JsonValue::Bool(*value),
        IpcValue::Integer(value) => JsonValue::from(*value),
        IpcValue::Number(value) => JsonValue::from(*value),
        IpcValue::String(value) => JsonValue::String(value.clone()),
    }
}

fn ipc_value_from_json(value: &JsonValue) -> io::Result<IpcValue> {
    match value {
        JsonValue::Null => Ok(IpcValue::Nil),
        JsonValue::Bool(value) => Ok(IpcValue::Boolean(*value)),
        JsonValue::Number(value) if value.is_i64() => {
            Ok(IpcValue::Integer(value.as_i64().unwrap()))
        }
        JsonValue::Number(value) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(IpcValue::Number)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid numeric IPC value")),
        JsonValue::String(value) => Ok(IpcValue::String(value.clone())),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC values must be primitive",
        )),
    }
}

