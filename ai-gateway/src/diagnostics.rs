use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fs::OpenOptions;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::types::ChatMessage;

static LOG_FILE_PATH: OnceLock<PathBuf> = OnceLock::new();
static CHAT_SIGNATURES: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
static PROVIDER_SIGNATURES: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticsLevel {
    Off,
    Summary,
    Delta,
    Trace,
}

pub fn set_log_file_path(path: impl AsRef<Path>) {
    let _ = LOG_FILE_PATH.set(path.as_ref().to_path_buf());
}

pub fn append_line(line: impl AsRef<str>) {
    let Some(path) = LOG_FILE_PATH.get() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let _ = writeln!(file, "[{ts}] {}", line.as_ref());
}

pub fn log_chat_messages(
    label: &str,
    model_id: u32,
    model_name: &str,
    api_format: &str,
    messages: &[ChatMessage],
    tool_count: usize,
) {
    let level = diagnostics_level();
    if level == DiagnosticsLevel::Off {
        return;
    }
    let signatures = messages.iter().map(chat_signature).collect::<Vec<_>>();
    let key = format!("chat:{label}:{model_id}:{model_name}:{api_format}");
    let changed_indexes = changed_indexes(
        CHAT_SIGNATURES.get_or_init(Default::default),
        &key,
        &signatures,
    );
    let empty_indexes = messages
        .iter()
        .enumerate()
        .filter_map(|(idx, msg)| {
            if msg.content.trim().is_empty()
                && msg
                    .tool_calls
                    .as_ref()
                    .map(|calls| calls.is_empty())
                    .unwrap_or(true)
            {
                Some(idx)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    append_line(format!(
        "[ai-gateway messages] level={:?} label={} model_id={} model={} api_format={} message_count={} tool_count={} empty_indexes={} changed_indexes={}",
        level,
        label,
        model_id,
        model_name,
        api_format,
        messages.len(),
        tool_count,
        join_indexes(empty_indexes.iter().copied()),
        join_indexes(changed_indexes.iter().copied())
    ));
    if level == DiagnosticsLevel::Summary {
        return;
    }
    for idx in detail_indexes(messages.len(), &changed_indexes, &empty_indexes, level) {
        let msg = &messages[idx];
        append_line(format!(
            "[ai-gateway message] label={} idx={} role={} content_len={} content_empty={} cache_control={} tool_calls={} tool_call_id={} name={} reasoning_len={} content_excerpt={}",
            label,
            idx,
            msg.role,
            msg.content.chars().count(),
            msg.content.trim().is_empty(),
            msg.cache_control,
            msg.tool_calls.as_ref().map(|calls| calls.len()).unwrap_or(0),
            msg.tool_call_id.as_deref().unwrap_or(""),
            msg.name.as_deref().unwrap_or(""),
            msg.reasoning_content.as_ref().map(|value| value.chars().count()).unwrap_or(0),
            diagnostic_excerpt(&msg.content, 240)
        ));
    }
}

pub fn log_provider_messages(
    label: &str,
    provider_api: &str,
    model: &str,
    url: &str,
    stream: bool,
    messages: &[Value],
    tool_count: usize,
) {
    let level = diagnostics_level();
    if level == DiagnosticsLevel::Off {
        return;
    }
    let signatures = messages.iter().map(provider_signature).collect::<Vec<_>>();
    let key = format!("provider:{label}:{provider_api}:{model}:{url}:stream={stream}");
    let changed_indexes = changed_indexes(
        PROVIDER_SIGNATURES.get_or_init(Default::default),
        &key,
        &signatures,
    );
    let empty_indexes = messages
        .iter()
        .enumerate()
        .filter_map(|(idx, msg)| {
            if provider_content_is_empty_or_null(msg.get("content")) {
                Some(idx)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    append_line(format!(
        "[ai-gateway provider-request] level={:?} label={} provider_api={} stream={} model={} url={} message_count={} tool_count={} empty_or_null_indexes={} changed_indexes={}",
        level,
        label,
        provider_api,
        stream,
        model,
        url,
        messages.len(),
        tool_count,
        join_indexes(empty_indexes.iter().copied()),
        join_indexes(changed_indexes.iter().copied())
    ));
    if level == DiagnosticsLevel::Summary {
        return;
    }
    for idx in detail_indexes(messages.len(), &changed_indexes, &empty_indexes, level) {
        let msg = &messages[idx];
        let content = msg.get("content");
        append_line(format!(
            "[ai-gateway provider-message] label={} idx={} role={} content_kind={} content_len={} content_empty_or_null={} tool_calls={} tool_call_id={} name={} content_excerpt={}",
            label,
            idx,
            msg.get("role").and_then(Value::as_str).unwrap_or("<missing>"),
            provider_content_kind(content),
            provider_content_len(content),
            provider_content_is_empty_or_null(content),
            msg.get("tool_calls").and_then(Value::as_array).map(|values| values.len()).unwrap_or(0),
            msg.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),
            msg.get("name").and_then(Value::as_str).unwrap_or(""),
            provider_content_excerpt(content, 240)
        ));
    }
}

fn diagnostics_level() -> DiagnosticsLevel {
    let value = std::env::var("AI_GATEWAY_DIAGNOSTICS_LEVEL")
        .or_else(|_| std::env::var("AI_GATEWAY_DIAGNOSTICS"))
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match value.as_str() {
        "" => DiagnosticsLevel::Off,
        "off" | "0" | "false" => DiagnosticsLevel::Off,
        "summary" => DiagnosticsLevel::Summary,
        "delta" | "1" | "true" => DiagnosticsLevel::Delta,
        "trace" | "full" => DiagnosticsLevel::Trace,
        _ => DiagnosticsLevel::Off,
    }
}

fn changed_indexes(
    store: &Mutex<HashMap<String, Vec<String>>>,
    key: &str,
    current: &[String],
) -> Vec<usize> {
    let Ok(mut guard) = store.lock() else {
        return (0..current.len()).collect();
    };
    let previous = guard.get(key);
    let changed = current
        .iter()
        .enumerate()
        .filter_map(|(idx, signature)| {
            if previous.and_then(|values| values.get(idx)) == Some(signature) {
                None
            } else {
                Some(idx)
            }
        })
        .collect::<Vec<_>>();
    guard.insert(key.to_string(), current.to_vec());
    changed
}

fn detail_indexes(
    message_count: usize,
    changed_indexes: &[usize],
    anomaly_indexes: &[usize],
    level: DiagnosticsLevel,
) -> Vec<usize> {
    if level == DiagnosticsLevel::Trace {
        return (0..message_count).collect();
    }
    let mut indexes = changed_indexes
        .iter()
        .chain(anomaly_indexes.iter())
        .copied()
        .collect::<Vec<_>>();
    indexes.sort_unstable();
    indexes.dedup();
    indexes
}

fn chat_signature(msg: &ChatMessage) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        msg.role,
        msg.content.chars().count(),
        msg.content.trim().is_empty(),
        msg.cache_control,
        msg.tool_calls
            .as_ref()
            .map(|calls| calls.len())
            .unwrap_or(0),
        msg.tool_call_id.as_deref().unwrap_or(""),
        msg.name.as_deref().unwrap_or(""),
        stable_hash(&msg.content)
    )
}

fn provider_signature(msg: &Value) -> String {
    let content = msg.get("content");
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        msg.get("role")
            .and_then(Value::as_str)
            .unwrap_or("<missing>"),
        provider_content_kind(content),
        provider_content_len(content),
        provider_content_is_empty_or_null(content),
        msg.get("tool_calls")
            .and_then(Value::as_array)
            .map(|values| values.len())
            .unwrap_or(0),
        msg.get("tool_call_id")
            .and_then(Value::as_str)
            .unwrap_or(""),
        stable_hash(&content.map(Value::to_string).unwrap_or_default())
    )
}

fn stable_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn join_indexes(indexes: impl Iterator<Item = usize>) -> String {
    let values = indexes.map(|idx| idx.to_string()).collect::<Vec<_>>();
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(",")
    }
}

fn provider_content_kind(content: Option<&Value>) -> &'static str {
    match content {
        None => "missing",
        Some(Value::Null) => "null",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
        Some(Value::Bool(_)) => "bool",
        Some(Value::Number(_)) => "number",
    }
}

fn provider_content_len(content: Option<&Value>) -> usize {
    match content {
        Some(Value::String(value)) => value.chars().count(),
        Some(Value::Array(values)) => values.len(),
        Some(Value::Object(values)) => values.len(),
        _ => 0,
    }
}

fn provider_content_is_empty_or_null(content: Option<&Value>) -> bool {
    match content {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.trim().is_empty(),
        Some(Value::Array(values)) => values.is_empty(),
        _ => false,
    }
}

fn provider_content_excerpt(content: Option<&Value>, max_chars: usize) -> String {
    match content {
        Some(Value::String(value)) => diagnostic_excerpt(value, max_chars),
        Some(Value::Array(values)) => {
            let summary = values
                .iter()
                .take(6)
                .map(|value| {
                    value
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or(value_type_name(value))
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(",");
            diagnostic_excerpt(
                &format!("array(len={}, types=[{}])", values.len(), summary),
                max_chars,
            )
        }
        Some(value) => diagnostic_excerpt(&value.to_string(), max_chars),
        None => "<missing>".to_string(),
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn diagnostic_excerpt(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut out = normalized.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}
