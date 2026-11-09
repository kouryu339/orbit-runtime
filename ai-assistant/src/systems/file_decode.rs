//! Private host decoding channel. No tool registration, model credentials or file paths as authority.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        RwLock,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderRegistration {
    pub id: String,
    pub version: String,
    pub extensions: Vec<String>,
    pub representations: Vec<String>,
    pub mode: String,
    pub endpoint: SocketAddr,
    pub token: String,
}

#[derive(Default)]
pub struct DecoderRegistry {
    entries: RwLock<BTreeMap<String, DecoderRegistration>>,
    frozen: AtomicBool,
}

impl std::fmt::Debug for DecoderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecoderRegistry")
            .field("frozen", &self.frozen.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

pub struct Decoded {
    pub bytes: Vec<u8>,
    pub revision: String,
    pub locations: Vec<Value>,
    pub decoder: Option<String>,
}

fn revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

impl DecoderRegistry {
    pub fn freeze(&self) {
        let _guard = self.entries.read().ok();
        self.frozen.store(true, Ordering::Release);
    }

    pub fn register(&self, mut registration: DecoderRegistration) -> Result<(), String> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| "decoder registry poisoned")?;
        if self.frozen.load(Ordering::Acquire) {
            return Err("file_decoders_frozen: register before creating conversations".into());
        }
        if registration.mode != "replace"
            || registration.id.is_empty()
            || registration.id.len() > 128
            || registration.version.is_empty()
            || registration.version.len() > 128
            || !registration.endpoint.ip().is_loopback()
            || registration.endpoint.port() == 0
            || registration.token.len() != 64
            || !registration.token.bytes().all(|b| b.is_ascii_hexdigit())
            || registration.extensions.is_empty()
            || registration.extensions.len() > 32
            || registration.representations.is_empty()
            || registration
                .representations
                .iter()
                .any(|r| r != "text" && r != "json")
        {
            return Err("invalid file decoder registration".into());
        }
        let mut extensions = std::collections::BTreeSet::new();
        for extension in &registration.extensions {
            let extension = extension.to_ascii_lowercase();
            if !extension.starts_with('.')
                || extension.len() < 2
                || extension.len() > 32
                || !extension[1..]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || entries.contains_key(&extension)
                || !extensions.insert(extension)
            {
                return Err("duplicate or invalid file decoder extension".into());
            }
        }
        registration.extensions = extensions.iter().cloned().collect();
        for extension in extensions {
            entries.insert(extension, registration.clone());
        }
        Ok(())
    }

    pub fn decode(
        &self,
        path: &Path,
        bytes: Vec<u8>,
        representation: &str,
        conversation_id: Option<&str>,
        cancel: &AtomicBool,
        deadline: Instant,
        max_decoded_bytes: usize,
    ) -> Result<Decoded, String> {
        self.freeze();
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .map(|v| format!(".{}", v.to_ascii_lowercase()))
            .unwrap_or_default();
        let registration = self
            .entries
            .read()
            .map_err(|_| "decoder registry poisoned")?
            .get(&extension)
            .cloned();
        let Some(registration) = registration else {
            let revision = if representation == "json" {
                revision(&bytes)
            } else {
                String::new()
            };
            return Ok(Decoded {
                bytes,
                revision,
                locations: vec![],
                decoder: None,
            });
        };
        let source_revision = revision(&bytes);
        if !registration
            .representations
            .iter()
            .any(|v| v == representation)
        {
            return Err(format!(
                "decoder_unsupported_view: {} does not provide {representation}",
                registration.id
            ));
        }
        let check = || {
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                Err("decoder_timeout_or_canceled".to_string())
            } else {
                Ok(())
            }
        };
        check()?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("decoder_input_limit".into());
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        let request = json!({
            "schema":"orbit-file-decode/v1", "request_id":request_id, "token":registration.token,
            "decoder_id":registration.id, "decoder_version":registration.version,
            "conversation_id":conversation_id, "name":path.to_string_lossy(), "extension":extension,
            "source_revision":source_revision, "bytes_base64":STANDARD.encode(&bytes),
            "representation":representation, "max_decoded_bytes":max_decoded_bytes,
            "remaining_ms":deadline.saturating_duration_since(Instant::now()).as_millis() as u64
        });
        let request = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        let mut stream = TcpStream::connect_timeout(
            &registration.endpoint,
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(500)),
        )
        .map_err(|_| "decoder_connection_failed".to_string())?;
        stream
            .set_write_timeout(Some(
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ))
            .map_err(|e| e.to_string())?;
        stream
            .write_all(registration.token.as_bytes())
            .map_err(|_| "decoder_request_failed")?;
        stream
            .write_all(
                &(u32::try_from(request.len()).map_err(|_| "decoder_request_too_large")?)
                    .to_be_bytes(),
            )
            .and_then(|_| stream.write_all(&request))
            .map_err(|_| "decoder_request_failed")?;
        stream
            .set_read_timeout(Some(Duration::from_millis(20)))
            .map_err(|e| e.to_string())?;
        let mut read = |target: &mut [u8]| -> Result<(), String> {
            let mut offset = 0;
            while offset < target.len() {
                check()?;
                match stream.read(&mut target[offset..]) {
                    Ok(0) => return Err("decoder_connection_closed".into()),
                    Ok(n) => offset += n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => return Err("decoder_response_failed".into()),
                }
            }
            Ok(())
        };
        let mut header = [0; 4];
        read(&mut header)?;
        let size = u32::from_be_bytes(header) as usize;
        if size > max_decoded_bytes.saturating_add(65536) {
            return Err("decoder_output_limit".into());
        }
        let mut response = vec![0; size];
        read(&mut response)?;
        check()?;
        let response: Value =
            serde_json::from_slice(&response).map_err(|_| "decoder_invalid_response")?;
        if response["schema"] != "orbit-file-decode-result/v1"
            || response["request_id"] != request_id
            || response["source_revision"] != source_revision
        {
            return Err("decoder_response_identity_mismatch".into());
        }
        if let Some(error) = response["error"].as_str() {
            return Err(format!(
                "decoder_failed: {}: {}",
                registration.id,
                error.chars().take(512).collect::<String>()
            ));
        }
        if response["complete"] != true {
            return Err("decoder_incomplete: host did not return a complete view".into());
        }
        let value = &response["value"];
        let decoded = if representation == "text" {
            value
                .as_str()
                .ok_or("decoder_invalid_text")?
                .as_bytes()
                .to_vec()
        } else {
            serde_json::to_vec(value).map_err(|e| e.to_string())?
        };
        if decoded.len() > max_decoded_bytes {
            return Err("decoder_output_limit".into());
        }
        let locations = response["locations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let revision = revision(
            &serde_json::to_vec(&json!([
                source_revision,
                registration.id,
                registration.version,
                representation
            ]))
            .map_err(|e| e.to_string())?,
        );
        Ok(Decoded {
            bytes: decoded,
            revision,
            locations,
            decoder: Some(registration.id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn registration(extension: &str) -> DecoderRegistration {
        DecoderRegistration {
            id: "host.fixture".into(),
            version: "1".into(),
            extensions: vec![extension.into()],
            representations: vec!["text".into(), "json".into()],
            mode: "replace".into(),
            endpoint: "127.0.0.1:12345".parse().unwrap(),
            token: "a".repeat(64),
        }
    }

    #[test]
    fn defaults_remain_without_registration_and_when_unrelated_formats_are_added() {
        for customized in [false, true] {
            let registry = DecoderRegistry::default();
            if customized {
                registry.register(registration(".pdf")).unwrap();
            }
            for (name, bytes, view) in [
                ("a.txt", b"hello".as_slice(), "text"),
                ("a.md", b"# heading".as_slice(), "text"),
                ("a.py", b"print('read only')".as_slice(), "text"),
                ("a.log", b"log".as_slice(), "text"),
                ("a.json", b"{\"id\":\"005\",\"value\":5}".as_slice(), "json"),
            ] {
                let decoded = registry
                    .decode(
                        Path::new(name),
                        bytes.to_vec(),
                        view,
                        Some("conv"),
                        &AtomicBool::new(false),
                        Instant::now() + Duration::from_secs(1),
                        1024,
                    )
                    .unwrap();
                assert_eq!(decoded.bytes, bytes);
                if view == "json" {
                    assert_eq!(decoded.revision, revision(bytes));
                }
                assert!(decoded.decoder.is_none());
            }
        }
    }

    #[test]
    fn registration_is_explicit_atomic_case_insensitive_and_frozen_before_use() {
        let registry = DecoderRegistry::default();
        let mut invalid = registration(".txt");
        invalid.mode = "fallback".into();
        assert!(registry.register(invalid).is_err());
        registry.register(registration(".TXT")).unwrap();
        assert!(registry.register(registration(".txt")).is_err());
        let mut batch = registration(".pdf");
        batch.extensions.push(".txt".into());
        assert!(registry.register(batch).is_err());
        assert!(!registry.entries.read().unwrap().contains_key(".pdf"));
        let mut remote = registration(".pdf");
        remote.endpoint = "192.0.2.1:80".parse().unwrap();
        assert!(registry.register(remote).is_err());
        registry.freeze();
        assert!(registry.register(registration(".pdf")).is_err());
    }

    #[test]
    fn host_failures_do_not_fall_back_to_plaintext() {
        let registry = DecoderRegistry::default();
        registry.register(registration(".txt")).unwrap();
        let error = registry
            .decode(
                Path::new("x.txt"),
                b"must not leak as fallback".to_vec(),
                "text",
                Some("conv"),
                &AtomicBool::new(true),
                Instant::now() + Duration::from_secs(1),
                1024,
            )
            .err()
            .unwrap();
        assert!(error.contains("canceled"));
    }

    fn with_host(
        version: &str,
        representation: &str,
        response: impl FnOnce(&Value) -> Value + Send + 'static,
    ) -> Result<Decoded, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut registration = registration(".txt");
        registration.endpoint = listener.local_addr().unwrap();
        registration.version = version.into();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut token = [0; 64];
            stream.read_exact(&mut token).unwrap();
            let mut header = [0; 4];
            stream.read_exact(&mut header).unwrap();
            let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
            stream.read_exact(&mut bytes).unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["conversation_id"], "trusted-conversation");
            assert_eq!(
                STANDARD
                    .decode(request["bytes_base64"].as_str().unwrap())
                    .unwrap(),
                b"original"
            );
            let bytes = serde_json::to_vec(&response(&request)).unwrap();
            stream
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .unwrap();
            stream.write_all(&bytes).unwrap();
        });
        let registry = DecoderRegistry::default();
        registry.register(registration).unwrap();
        let decoded = registry.decode(
            Path::new("x.TXT"),
            b"original".to_vec(),
            representation,
            Some("trusted-conversation"),
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(2),
            1024,
        );
        worker.join().unwrap();
        decoded
    }

    fn response(request: &Value) -> Value {
        json!({"schema":"orbit-file-decode-result/v1","request_id":request["request_id"],
            "source_revision":request["source_revision"],"complete":true,
            "value":if request["representation"]=="text" {json!("custom text")} else {json!({"id":"005","number":5})}})
    }

    #[test]
    fn overrides_text_and_json_and_invalidates_revision_on_decoder_version_change() {
        let first = with_host("1", "text", response).unwrap();
        assert_eq!(first.bytes, b"custom text");
        assert_eq!(first.decoder.as_deref(), Some("host.fixture"));
        let second = with_host("2", "text", response).unwrap();
        assert_ne!(first.revision, second.revision);
        let json = with_host("1", "json", response).unwrap();
        let value: Value = serde_json::from_slice(&json.bytes).unwrap();
        assert_eq!(value["id"], "005");
        assert_eq!(value["number"], 5);
        assert_ne!(first.revision, json.revision);
    }

    #[test]
    fn rejects_wrong_identity_incomplete_views_and_output_overflow() {
        assert!(with_host("1", "text", |r| {
            let mut v = response(r);
            v["source_revision"] = json!("other");
            v
        })
        .err()
        .unwrap()
        .contains("identity"));
        assert!(with_host("1", "text", |r| {
            let mut v = response(r);
            v["complete"] = json!(false);
            v
        })
        .err()
        .unwrap()
        .contains("incomplete"));
        assert!(with_host("1", "text", |r| {
            let mut v = response(r);
            v["value"] = json!("x".repeat(2048));
            v
        })
        .err()
        .unwrap()
        .contains("output_limit"));
    }

    #[test]
    fn waiting_for_host_obeys_deadline_and_cancellation() {
        for canceled in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let mut registration = registration(".txt");
            registration.endpoint = listener.local_addr().unwrap();
            let flag = std::sync::Arc::new(AtomicBool::new(false));
            let cancel = flag.clone();
            let worker = std::thread::spawn(move || {
                let (_stream, _) = listener.accept().unwrap();
                std::thread::sleep(Duration::from_millis(30));
                if canceled {
                    cancel.store(true, Ordering::Release);
                }
                std::thread::sleep(Duration::from_millis(70));
            });
            let registry = DecoderRegistry::default();
            registry.register(registration).unwrap();
            let deadline = Instant::now() + Duration::from_millis(if canceled { 1000 } else { 20 });
            assert!(registry
                .decode(
                    Path::new("a.txt"),
                    b"original".to_vec(),
                    "text",
                    Some("conv"),
                    &flag,
                    deadline,
                    1024
                )
                .err()
                .unwrap()
                .contains("timeout_or_canceled"));
            worker.join().unwrap();
        }
    }
}
