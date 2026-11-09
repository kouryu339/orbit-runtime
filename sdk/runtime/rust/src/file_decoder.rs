//! Host-owned decoder bridge. The private channel is independent of serialized Runtime invokes.
use crate::{Result, RuntimeError};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct FileDecoder {
    pub id: String,
    pub version: String,
    pub extensions: Vec<String>,
    pub representations: Vec<String>,
    handler: Arc<
        dyn Fn(&FileDecodeRequest) -> std::result::Result<FileDecodeOutput, String> + Send + Sync,
    >,
}

impl FileDecoder {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        extensions: Vec<String>,
        representations: Vec<String>,
        handler: impl Fn(&FileDecodeRequest) -> std::result::Result<FileDecodeOutput, String>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            extensions,
            representations,
            handler: Arc::new(handler),
        }
    }
}

pub struct FileDecodeRequest {
    pub conversation_id: Option<String>,
    /// Display and location only. Decode bytes; do not reopen this name as a path.
    pub name: String,
    pub extension: String,
    pub bytes: Vec<u8>,
    pub source_revision: String,
    pub representation: String,
    pub max_decoded_bytes: usize,
    pub deadline: Instant,
}

pub struct FileDecodeOutput {
    pub value: Value,
    pub locations: Vec<Value>,
}
impl FileDecodeOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            value: json!(text.into()),
            locations: vec![],
        }
    }
    pub fn json(value: Value) -> Self {
        Self {
            value,
            locations: vec![],
        }
    }
}

pub(crate) struct DecoderBridge {
    registrations: Vec<Value>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DecoderBridge {
    pub(crate) fn start(decoders: Vec<FileDecoder>) -> Result<Option<Self>> {
        if decoders.is_empty() {
            return Ok(None);
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| RuntimeError::new(format!("decoder listener: {e}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| RuntimeError::new(e.to_string()))?;
        let endpoint = listener
            .local_addr()
            .map_err(|e| RuntimeError::new(e.to_string()))?
            .to_string();
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|e| RuntimeError::new(format!("decoder secret: {e}")))?;
        let token = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let mut registrations = vec![];
        let mut handlers = BTreeMap::new();
        for decoder in decoders {
            registrations.push(json!({"id":decoder.id,"version":decoder.version,"extensions":decoder.extensions,
                "representations":decoder.representations,"mode":"replace","endpoint":endpoint,"token":token}));
            if handlers.insert(decoder.id.clone(), decoder).is_some() {
                return Err(RuntimeError::new("duplicate decoder ID"));
            }
        }
        let handlers = Arc::new(handlers);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let active = Arc::new(AtomicUsize::new(0));
        let thread = thread::Builder::new()
            .name("orbit-file-decoders".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if active.fetch_add(1, Ordering::AcqRel) >= 4 {
                                active.fetch_sub(1, Ordering::AcqRel);
                                continue;
                            }
                            let handlers = handlers.clone();
                            let token = token.clone();
                            let active = active.clone();
                            let worker_active = active.clone();
                            if thread::Builder::new()
                                .name("orbit-file-decode".into())
                                .spawn(move || {
                                    struct Active(Arc<AtomicUsize>);
                                    impl Drop for Active {
                                        fn drop(&mut self) {
                                            self.0.fetch_sub(1, Ordering::AcqRel);
                                        }
                                    }
                                    let _active = Active(worker_active);
                                    let _ = serve(stream, &token, &handlers);
                                })
                                .is_err()
                            {
                                active.fetch_sub(1, Ordering::AcqRel);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| RuntimeError::new(e.to_string()))?;
        Ok(Some(Self {
            registrations,
            stop,
            thread: Some(thread),
        }))
    }
    pub(crate) fn registrations(&self) -> &[Value] {
        &self.registrations
    }
}
impl Drop for DecoderBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    mut stream: TcpStream,
    token: &str,
    handlers: &BTreeMap<String, FileDecoder>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut supplied = [0u8; 64];
    stream.read_exact(&mut supplied)?;
    if supplied
        .iter()
        .zip(token.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (*a ^ *b))
        != 0
    {
        return Err(std::io::Error::other("unauthorized decoder request"));
    }
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size > 96 * 1024 * 1024 {
        return Err(std::io::Error::other("decoder request limit"));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    let request: Value = serde_json::from_slice(&bytes)?;
    let supplied = request["token"].as_str().unwrap_or("").as_bytes();
    if supplied.len() != token.len()
        || supplied
            .iter()
            .zip(token.as_bytes())
            .fold(0u8, |acc, (a, b)| acc | (*a ^ *b))
            != 0
        || request["schema"] != "orbit-file-decode/v1"
    {
        return Err(std::io::Error::other("unauthorized decoder request"));
    }
    let decoder = handlers
        .get(request["decoder_id"].as_str().unwrap_or(""))
        .ok_or_else(|| std::io::Error::other("unknown decoder"))?;
    if request["decoder_version"] != decoder.version {
        return Err(std::io::Error::other("decoder version mismatch"));
    }
    let representation = request["representation"].as_str().unwrap_or("");
    let max = request["max_decoded_bytes"].as_u64().unwrap_or(0) as usize;
    if max == 0
        || max > 64 * 1024 * 1024
        || !decoder.representations.iter().any(|r| r == representation)
    {
        return Err(std::io::Error::other("invalid decode view or limits"));
    }
    let remaining = request["remaining_ms"].as_u64().unwrap_or(0).min(60000);
    let input = FileDecodeRequest {
        conversation_id: request["conversation_id"].as_str().map(str::to_owned),
        name: request["name"].as_str().unwrap_or("").into(),
        extension: request["extension"].as_str().unwrap_or("").into(),
        bytes: STANDARD
            .decode(request["bytes_base64"].as_str().unwrap_or(""))
            .map_err(std::io::Error::other)?,
        source_revision: request["source_revision"].as_str().unwrap_or("").into(),
        representation: representation.into(),
        max_decoded_bytes: max,
        deadline: Instant::now() + Duration::from_millis(remaining),
    };
    let output =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (decoder.handler)(&input)))
            .unwrap_or_else(|_| Err("decoder panicked".into()));
    let mut response = json!({"schema":"orbit-file-decode-result/v1","request_id":request["request_id"],"source_revision":input.source_revision});
    match output {
        Ok(output) if Instant::now() < input.deadline => {
            if (representation == "text" && !output.value.is_string())
                || serde_json::to_vec(&output.value)?.len() > max
            {
                response["error"] = json!("invalid or oversized decoded view");
            } else {
                response["value"] = output.value;
                response["locations"] = json!(output.locations);
                response["complete"] = json!(true);
            }
        }
        Ok(_) => response["error"] = json!("decoder deadline exceeded"),
        Err(error) => response["error"] = json!(error.chars().take(512).collect::<String>()),
    }
    let mut bytes = serde_json::to_vec(&response)?;
    if bytes.len() > max.saturating_add(65536) {
        response.as_object_mut().unwrap().remove("value");
        response.as_object_mut().unwrap().remove("locations");
        response["error"] = json!("decoder response limit");
        bytes = serde_json::to_vec(&response)?;
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)
}
