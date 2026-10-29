//! Structured JSON inspection over the same host-owned authority as Grep.
use super::grep::GrepService;
use async_trait::async_trait;
use corework::{
    ai_system::{AIInput, AIOutput},
    define_operation,
    error::FrameworkError,
    orchestration::Context,
    system::SystemOperation,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Hash the exact bytes read, including whitespace. Hosts must use this algorithm
/// under their write lock before applying a pointer-based edit.
pub fn document_revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Default)]
struct Request {
    root_id: String,
    path: String,
    action: String,
    pointer: String,
    field: Option<String>,
    value_json: Option<String>,
    contains: Option<String>,
    offset: usize,
    limit: usize,
    expected_revision: String,
}

struct Budget<'a> {
    cancel: &'a AtomicBool,
    started: Instant,
    timeout: Duration,
}
impl Budget<'_> {
    fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) || self.started.elapsed() >= self.timeout {
            Err("timeout_or_canceled: narrow pointer or query".into())
        } else {
            Ok(())
        }
    }
}
struct LimitedWriter {
    used: usize,
    limit: usize,
}
impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.used = self.used.saturating_add(bytes.len());
        if self.used > self.limit {
            return Err(std::io::Error::other("output budget"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn fits(value: &Value, limit: usize) -> bool {
    serde_json::to_writer(LimitedWriter { used: 0, limit }, value).is_ok()
}
fn child_pointer(parent: &str, key: &str) -> String {
    format!("{}/{}", parent, key.replace('~', "~0").replace('/', "~1"))
}
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
fn descriptor(value: &Value, pointer: &str) -> Value {
    let mut result = json!({"pointer":pointer,"type":kind(value),"value_complete":false});
    match value {
        Value::Array(v) => result["count"] = json!(v.len()),
        Value::Object(v) => result["count"] = json!(v.len()),
        Value::String(v) if v.len() > 256 => {
            result["characters"] = json!(v.chars().count());
            result["preview"] = json!(v.chars().take(80).collect::<String>());
        }
        _ => {
            if fits(value, 256) {
                result["value"] = value.clone();
                result["value_complete"] = json!(true);
            }
        }
    }
    result
}
fn validate_pointer(pointer: &str) -> Result<(), String> {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return Err("pointer must be empty (root) or start with /".into());
    }
    let mut chars = pointer.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return Err("pointer has invalid ~ escape; use ~0 or ~1".into());
        }
    }
    Ok(())
}

mod parser;

fn query(service: &GrepService, req: &Request, cancel: &AtomicBool) -> Result<Value, String> {
    let config = &service.config;
    let budget = Budget {
        cancel,
        started: Instant::now(),
        timeout: Duration::from_millis(config.timeout_ms),
    };
    if req.root_id.is_empty() {
        let result = json!({"roots":service.authorized_ids(),"note":"GrepJson shares Grep roots and limits. Choose root_id and a relative JSON file path."});
        return if fits(&result, config.max_output_bytes) {
            Ok(result)
        } else {
            Err("root list exceeds output budget".into())
        };
    }
    if !matches!(req.action.as_str(), "outline" | "search" | "read" | "page") {
        return Err("action must be outline, search, read, or page".into());
    }
    if req.limit == 0 {
        return Err("limit must be positive".into());
    }
    if req.offset > 0 && req.expected_revision.is_empty() {
        return Err("expected_revision is required for continuation offsets".into());
    }
    validate_pointer(&req.pointer)?;
    let expected_value = req
        .value_json
        .as_deref()
        .map(|raw| {
            parser::parse_document(raw.as_bytes(), config.max_entries, &budget)
                .map_err(|e| format!("invalid value_json: {e}"))
        })
        .transpose()?;
    let bytes = service.read_authorized_file(&req.root_id, &req.path)?;
    budget.check()?;
    let revision = document_revision(&bytes);
    if !req.expected_revision.is_empty() && req.expected_revision != revision {
        return Err(format!(
            "revision_conflict: current_revision={revision}; read again before using old pointers"
        ));
    }
    let doc = parser::parse_document(&bytes, config.max_entries, &budget)?;
    budget.check()?;
    let node = doc.pointer(&req.pointer).ok_or("pointer_not_found")?;
    let mut result = json!({
        "root_id":req.root_id,"path":req.path,"revision":revision,
        "pointer":req.pointer,"action":req.action,"complete":true
    });
    let max = config.max_output_bytes;
    if !fits(&result, max / 2) {
        return Err(
            "request paths exceed output budget; shorten path or increase host budget".into(),
        );
    }
    if req.action == "read" && fits(node, max / 2) {
        result["value"] = node.clone();
        result["value_complete"] = json!(true);
    } else if req.action == "search" {
        if req.field.is_none() && req.value_json.is_none() && req.contains.is_none() {
            return Err("search requires field, value_json or contains".into());
        }
        result["matches"] = json!([]);
        let mut seen = 0;
        let scoped_key = req.pointer.rsplit_once('/').and_then(|(parent, key)| {
            doc.pointer(parent)
                .filter(|v| v.is_object())
                .map(|_| key.replace("~1", "/").replace("~0", "~"))
        });
        search(
            node,
            &req.pointer,
            scoped_key.as_deref(),
            &doc,
            req,
            expected_value.as_ref(),
            &budget,
            &mut seen,
            &mut result,
            config.max_results.min(req.limit),
            max,
        )?;
        if result["complete"] == true {
            result["total_matches"] = json!(seen);
        }
    } else {
        result["value_complete"] = json!(false);
        if req.action == "read" {
            result["reason"] = json!("subtree_exceeds_output_budget");
        }
        page(
            node,
            req,
            &budget,
            &mut result,
            config.max_results.min(req.limit),
            max,
        )?;
    }
    if !fits(&result, max) {
        return Err(
            "output budget too small for metadata; narrow pointer or increase host budget".into(),
        );
    }
    Ok(result)
}

fn push(result: &mut Value, list: &str, item: Value, next_offset: usize, max: usize) -> bool {
    result[list].as_array_mut().unwrap().push(item);
    // Reserve space for continuation metadata while testing the entire output envelope.
    if fits(result, max.saturating_sub(160)) {
        return true;
    }
    result[list].as_array_mut().unwrap().pop();
    result["complete"] = json!(false);
    result["next_offset"] = json!(next_offset);
    result["reason"] = json!("max_output_bytes");
    false
}

fn page(
    node: &Value,
    req: &Request,
    budget: &Budget<'_>,
    result: &mut Value,
    limit: usize,
    max: usize,
) -> Result<(), String> {
    if let Value::String(text) = node {
        let total = text.chars().count();
        if req.offset > total {
            return Err("offset exceeds string length".into());
        }
        let mut chunk = String::new();
        result["offset_unit"] = json!("unicode_scalar");
        result["total"] = json!(total);
        result["chunk"] = json!("");
        let mut next = req.offset;
        for c in text.chars().skip(req.offset).take(limit) {
            budget.check()?;
            chunk.push(c);
            result["chunk"] = json!(chunk);
            if !fits(result, max.saturating_sub(160)) {
                chunk.pop();
                result["chunk"] = json!(chunk);
                break;
            }
            next += 1;
        }
        if next < total {
            if next == req.offset {
                return Err("output budget cannot hold one character".into());
            }
            result["complete"] = json!(false);
            result["next_offset"] = json!(next);
        }
        return Ok(());
    }
    let total = match node {
        Value::Array(v) => v.len(),
        Value::Object(v) => v.len(),
        _ => {
            if !fits(node, max / 2) {
                return Err("scalar exceeds output budget; increase host budget".into());
            }
            result["value"] = node.clone();
            result["value_complete"] = json!(true);
            return Ok(());
        }
    };
    if req.offset > total {
        return Err("offset exceeds child count".into());
    }
    result["total"] = json!(total);
    result["items"] = json!([]);
    let children: Box<dyn Iterator<Item = (String, &Value)> + '_> = match node {
        Value::Array(a) => Box::new(a.iter().enumerate().map(|(i, v)| (i.to_string(), v))),
        Value::Object(o) => Box::new(o.iter().map(|(k, v)| (k.clone(), v))),
        _ => unreachable!(),
    };
    let mut next = req.offset;
    for (key, value) in children.skip(req.offset).take(limit) {
        budget.check()?;
        let pointer = child_pointer(&req.pointer, &key);
        let summary = descriptor(value, &pointer);
        let item = if req.action != "outline" && fits(value, max / 2) {
            json!({"pointer":pointer,"value":value,"value_complete":true,"type":kind(value)})
        } else {
            summary.clone()
        };
        if !push(result, "items", item, next, max) {
            // Preserve full small values by moving them to the next page instead
            // of replacing the last one with a descriptor that needs another read.
            if !result["items"].as_array().unwrap().is_empty() {
                break;
            }
            // If a full child cannot fit even on an empty page, descend by Pointer.
            result["complete"] = json!(true);
            result.as_object_mut().unwrap().remove("next_offset");
            result.as_object_mut().unwrap().remove("reason");
            if !push(result, "items", summary, next, max) {
                break;
            }
        }
        next += 1;
    }
    if next < total {
        if next == req.offset {
            return Err(
                "output budget cannot hold a child descriptor; increase host budget".into(),
            );
        }
        result["complete"] = json!(false);
        result["next_offset"] = json!(next);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn search(
    node: &Value,
    pointer: &str,
    key: Option<&str>,
    doc: &Value,
    req: &Request,
    expected_value: Option<&Value>,
    budget: &Budget<'_>,
    seen: &mut usize,
    result: &mut Value,
    limit: usize,
    max: usize,
) -> Result<bool, String> {
    budget.check()?;
    let matched = req.field.as_deref().is_none_or(|f| key == Some(f))
        && expected_value.is_none_or(|v| node == v)
        && req
            .contains
            .as_deref()
            .is_none_or(|v| node.as_str().is_some_and(|s| s.contains(v)));
    if matched {
        if *seen >= req.offset {
            if result["matches"].as_array().unwrap().len() >= limit {
                result["complete"] = json!(false);
                result["next_offset"] = json!(*seen);
                return Ok(false);
            }
            let mut hit = descriptor(node, pointer);
            let parent = pointer.rsplit_once('/').map(|(p, _)| p);
            hit["parent_pointer"] = json!(parent);
            let mut ancestors = Vec::new();
            let mut current = parent;
            // Nearby context is a bounded preview; every ancestor remains accessible by Pointer.
            while let Some(p) = current {
                if let Some(v) = doc.pointer(p) {
                    let mut context = descriptor(v, p);
                    if let Some(object) = v.as_object() {
                        let mut identity = serde_json::Map::new();
                        for k in ["id", "name", "type"] {
                            if let Some(v) = object.get(k) {
                                identity.insert(k.into(), descriptor(v, &child_pointer(p, k)));
                            }
                        }
                        context["identity"] = Value::Object(identity);
                    }
                    ancestors.push(context);
                }
                if ancestors.len() == 3 {
                    break;
                }
                current = p.rsplit_once('/').map(|(p, _)| p);
            }
            hit["ancestors"] = json!(ancestors);
            hit["ancestor_context_limit"] = json!(3);
            if !push(result, "matches", hit, *seen, max) {
                if *seen == req.offset {
                    return Err("output budget cannot hold match context; narrow pointer or increase host budget".into());
                }
                return Ok(false);
            }
        }
        *seen += 1;
    }
    match node {
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                if !search(
                    v,
                    &child_pointer(pointer, &i.to_string()),
                    None,
                    doc,
                    req,
                    expected_value,
                    budget,
                    seen,
                    result,
                    limit,
                    max,
                )? {
                    return Ok(false);
                }
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                if !search(
                    v,
                    &child_pointer(pointer, k),
                    Some(k),
                    doc,
                    req,
                    expected_value,
                    budget,
                    seen,
                    result,
                    limit,
                    max,
                )? {
                    return Ok(false);
                }
            }
        }
        _ => {}
    }
    Ok(true)
}

#[define_operation(
    name = "GrepJson", display_name = "读取JSON{root_id}/{path}，操作{action}，节点{pointer}，字段{field}，值{value_json}，包含{contains}，偏移{offset}，数量{limit}，版本{expected_revision}", category = "Search", system_only,
    description = "Inspect JSON files in the same host-authorized roots as Grep. Omit root_id to list roots. outline lists child descriptors; search matches field/value_json/contains together and returns JSON Pointers with ancestor context; read returns a full value or explicit paged descriptors; page reads array/object children or Unicode string chunks. Resume with next_offset AND expected_revision. complete covers enumeration, value_complete marks exact values. Host writes must compare this SHA-256 revision under their write lock. Read-only; no Workflow node.",
    params {
        root_id: "String@Same authorized root ID as Grep; omit to list.",
        path: "String@Relative JSON file path, forward slashes. No absolute paths or traversal.",
        action: "String@outline (default), search, read, or page.",
        pointer: "String@JSON Pointer; empty means root. Escape ~ as ~0 and / as ~1.",
        field: "String@Search: exact object field name. Optional.",
        value_json: "String@Search: exact JSON value, e.g. quoted string, number, false or null. Optional.",
        contains: "String@Search: literal substring of a string value. Optional.",
        offset: "u64@Default 0. Search match offset, child index, or Unicode character offset. Nonzero requires expected_revision.",
        limit: "u64@Positive page size, capped by host max_results; default 50.",
        expected_revision: "String@Revision from previous result. Required for continuation; conflict requires reread."
    }, destructive = false, readonly = true, idempotent = true, open_world = false
)]
pub struct GrepJsonSystem;

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[async_trait]
impl SystemOperation for GrepJsonSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;
    fn name(&self) -> &str {
        "GrepJson"
    }
    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let service = match ctx.resolve_shared_component::<GrepService>() {
            Ok(v) => v,
            Err(_) => return Ok(AIOutput::error(403, "Host has not authorized Grep roots")),
        };
        let request = (|| -> Result<Request, String> {
            for name in [
                "path",
                "pointer",
                "field",
                "value_json",
                "contains",
                "expected_revision",
            ] {
                if args
                    .get(name)
                    .is_some_and(|s| s.len() > service.config.max_pattern_bytes)
                {
                    return Err(format!("{name} exceeds host max_pattern_bytes"));
                }
            }
            Ok(Request {
                root_id: args.get("root_id").unwrap_or("").into(),
                path: args.get("path").unwrap_or("").into(),
                action: args.get("action").unwrap_or("outline").into(),
                pointer: args.get("pointer").unwrap_or("").into(),
                field: args.get("field").map(str::to_owned),
                value_json: args.get("value_json").map(str::to_owned),
                contains: args.get("contains").map(str::to_owned),
                offset: args
                    .get("offset")
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| "offset must be nonnegative")?,
                limit: args
                    .get("limit")
                    .unwrap_or("50")
                    .parse()
                    .map_err(|_| "limit must be positive")?,
                expected_revision: args.get("expected_revision").unwrap_or("").into(),
            })
        })();
        let request = match request {
            Ok(v) => v,
            Err(e) => return Ok(AIOutput::error(400, e)),
        };
        let permit = match service.permits.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                return Ok(AIOutput::error(
                    429,
                    "Shared Grep/GrepJson concurrency limit reached",
                ))
            }
        };
        let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
        let flag = cancel.0.clone();
        let timeout = Duration::from_millis(service.config.timeout_ms);
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            query(&service, &request, &flag)
        });
        match corework::platform::time::timeout(timeout, task).await {
            Ok(Ok(Ok(value))) => {
                let text = value.to_string();
                Ok(AIOutput::success(value, text))
            }
            Ok(Ok(Err(e))) => Ok(AIOutput::error(
                if e.starts_with("revision_conflict:") {
                    409
                } else {
                    400
                },
                e,
            )),
            Ok(Err(e)) => Ok(AIOutput::error(500, format!("JSON worker failed: {e}"))),
            Err(_) => Ok(AIOutput::error(
                408,
                "JSON read timed out; narrow query or adjust host limits",
            )),
        }
    }
}

#[cfg(test)]
mod tests;
