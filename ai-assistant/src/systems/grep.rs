//! Host-authorized, AI-only file search. Authority is a shared component, never cache data.
use async_trait::async_trait;
use cap_std::fs::Dir;
use corework::{
    ai_system::{AIInput, AIOutput},
    define_operation,
    error::FrameworkError,
    orchestration::Context,
    system::SystemOperation,
};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrepRoot {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GrepConfig {
    pub roots: Vec<GrepRoot>,
    pub timeout_ms: u64,
    pub max_results: usize,
    pub max_output_bytes: usize,
    pub max_file_bytes: usize,
    pub max_pattern_bytes: usize,
    pub max_regex_bytes: usize,
    pub max_line_bytes: usize,
    pub max_entries: usize,
    pub max_concurrent_searches: usize,
}
impl Default for GrepConfig {
    fn default() -> Self {
        Self {
            roots: vec![],
            timeout_ms: 10_000,
            max_results: 200,
            max_output_bytes: 65_536,
            max_file_bytes: 2 * 1024 * 1024,
            max_pattern_bytes: 16 * 1024,
            max_regex_bytes: 8 * 1024 * 1024,
            max_line_bytes: 4096,
            max_entries: 100_000,
            max_concurrent_searches: 2,
        }
    }
}

#[derive(Debug)]
struct Root {
    dir: Dir,
    path: PathBuf,
}

#[derive(Debug)]
pub struct GrepService {
    config: GrepConfig,
    roots: BTreeMap<String, Root>,
    permits: Arc<tokio::sync::Semaphore>,
}
impl GrepService {
    /// Called by the host at registration. Open directory capabilities pin authority.
    pub fn new(mut config: GrepConfig, base: Option<&Path>) -> Result<Self, String> {
        if config.timeout_ms == 0
            || config.max_results == 0
            || config.max_output_bytes < 1024
            || config.max_file_bytes == 0
            || config.max_pattern_bytes == 0
            || config.max_regex_bytes == 0
            || config.max_line_bytes == 0
            || config.max_entries == 0
            || config.max_concurrent_searches == 0
            || config.max_concurrent_searches > tokio::sync::Semaphore::MAX_PERMITS
            || config.max_file_bytes == usize::MAX
        {
            return Err(
                "grep limits must be positive; max_output_bytes must be at least 1024".into(),
            );
        }
        let mut roots = BTreeMap::new();
        for root in &mut config.roots {
            if root.id.is_empty()
                || root.id.len() > 64
                || !root
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                || roots.contains_key(&root.id)
            {
                return Err(
                    "grep root IDs must be unique, 1-64 ASCII letters/digits/underscore/hyphen"
                        .into(),
                );
            }
            if !root.path.is_absolute() {
                root.path = base
                    .ok_or("relative grep root requires a resource base directory")?
                    .join(&root.path);
            }
            root.path = root
                .path
                .canonicalize()
                .map_err(|e| format!("grep root '{}': {e}", root.id))?;
            let dir = Dir::open_ambient_dir(&root.path, cap_std::ambient_authority())
                .map_err(|e| format!("grep root '{}': {e}", root.id))?;
            roots.insert(
                root.id.clone(),
                Root {
                    dir,
                    path: root.path.clone(),
                },
            );
        }
        Ok(Self {
            permits: Arc::new(tokio::sync::Semaphore::new(config.max_concurrent_searches)),
            config,
            roots,
        })
    }

    fn search(&self, request: Request, cancel: &AtomicBool) -> Result<Value, String> {
        let start = Instant::now();
        let expired = || {
            cancel.load(Ordering::Relaxed)
                || start.elapsed().as_millis() >= self.config.timeout_ms as u128
        };
        if request.root_id.is_empty() {
            let value = json!({"roots":self.roots.keys().collect::<Vec<_>>(), "note":"Choose root_id; paths are relative to that root."});
            if value.to_string().len() > self.config.max_output_bytes {
                return Err(
                    "root list exceeds host max_output_bytes; host must increase limit".into(),
                );
            }
            return Ok(value);
        }
        let root = self
            .roots
            .get(&request.root_id)
            .ok_or("root_id is not authorized")?;
        let relative = checked_relative(&request.path)?;
        if request.pattern.is_empty() {
            return Err("pattern must not be empty".into());
        }
        if request.pattern.len() > self.config.max_pattern_bytes
            || request.glob.len() > self.config.max_pattern_bytes
        {
            return Err("pattern or glob exceeds host max_pattern_bytes".into());
        }
        if request.limit == 0 {
            return Err("limit must be positive".into());
        }
        if !matches!(
            request.output_mode.as_str(),
            "content" | "files_with_matches" | "count"
        ) {
            return Err("output_mode must be content, files_with_matches or count".into());
        }
        // Only Rust regex syntax: no arbitrary preprocessor, shell or backtracking engine.
        let pattern = if request.literal {
            regex::escape(&request.pattern)
        } else {
            request.pattern.clone()
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(request.ignore_case)
            .size_limit(self.config.max_regex_bytes)
            .build()
            .map_err(|e| format!("invalid pattern: {e}"))?;
        let glob = if request.glob.is_empty() {
            None
        } else {
            Some(
                globset::Glob::new(&request.glob)
                    .map_err(|e| format!("invalid glob: {e}"))?
                    .compile_matcher(),
            )
        };
        // Validate every requested ancestor before traversal, including junctions on Windows.
        let mut ancestor = PathBuf::new();
        let mut rules = Vec::new();
        self.load_rules(root, Path::new(""), &mut rules)?;
        for part in relative.components() {
            ancestor.push(part);
            let meta = root
                .dir
                .symlink_metadata(&ancestor)
                .map_err(|e| format!("invalid search path: {e}"))?;
            if is_link(&meta) {
                return Err("search path contains a symbolic link or reparse point".into());
            }
            if ancestor != relative && meta.is_dir() {
                self.load_rules(root, &ancestor, &mut rules)?;
            }
        }
        let mut pending = vec![(relative, rules)];
        let mut result = SearchResult::default();
        let limit = request.limit.min(self.config.max_results);
        let mut entries = 0usize;
        let mut output_bytes = 0usize;
        'scan: while let Some((path, mut rules)) = pending.pop() {
            if expired() {
                result.stop("timeout_or_canceled");
                break;
            }
            entries += 1;
            if entries > self.config.max_entries {
                result.stop("max_entries");
                break;
            }
            let meta = if path.as_os_str().is_empty() {
                root.dir.dir_metadata()
            } else {
                root.dir.symlink_metadata(&path)
            };
            let meta = match meta {
                Ok(m) => m,
                Err(_) => {
                    result.skipped_io += 1;
                    continue;
                }
            };
            if is_link(&meta) {
                result.skipped_links += 1;
                continue;
            }
            if !path.as_os_str().is_empty()
                && (hidden(&path) || ignored(root, &path, meta.is_dir(), &rules))
            {
                continue;
            }
            if meta.is_dir() {
                if !path.as_os_str().is_empty() {
                    self.load_rules(root, &path, &mut rules)?;
                }
                // All opens are root-relative capabilities; a concurrent symlink swap cannot grant access outside root.
                let dir = if path.as_os_str().is_empty() {
                    root.dir.try_clone()
                } else {
                    root.dir.open_dir(&path)
                };
                let children = match dir.and_then(|d| d.entries()) {
                    Ok(v) => v,
                    Err(_) => {
                        result.skipped_io += 1;
                        continue;
                    }
                };
                for child in children {
                    if expired() {
                        result.stop("timeout_or_canceled");
                        break 'scan;
                    }
                    if entries.saturating_add(pending.len()) >= self.config.max_entries {
                        result.stop("max_entries");
                        break 'scan;
                    }
                    match child {
                        Ok(child) => pending.push((path.join(child.file_name()), rules.clone())),
                        Err(_) => result.skipped_io += 1,
                    }
                }
                continue;
            }
            if !meta.is_file() || glob.as_ref().is_some_and(|g| !g.is_match(&path)) {
                continue;
            }
            if meta.len() > self.config.max_file_bytes as u64 {
                result.skipped_large += 1;
                continue;
            }
            let bytes = match read_bounded(&root.dir, &path, self.config.max_file_bytes) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    result.skipped_large += 1;
                    continue;
                }
                Err(_) => {
                    result.skipped_io += 1;
                    continue;
                }
            };
            let text = match std::str::from_utf8(&bytes) {
                Ok(t) if !bytes.contains(&0) => t,
                _ => {
                    result.skipped_binary += 1;
                    continue;
                }
            };
            result.searched_files += 1;
            let mut count = 0usize;
            for (index, line) in text.lines().enumerate() {
                if expired() {
                    result.stop("timeout_or_canceled");
                    break 'scan;
                }
                if !regex.is_match(line) {
                    continue;
                }
                count += 1;
                if request.output_mode == "count" {
                    continue;
                }
                let item = if request.output_mode == "files_with_matches" {
                    json!({"path":path})
                } else {
                    let (snippet, cut) = clip(line, self.config.max_line_bytes);
                    json!({"path":path,"line":index+1,"text":snippet,"line_truncated":cut})
                };
                if !result.push(item, limit, self.config.max_output_bytes, &mut output_bytes) {
                    break 'scan;
                }
                if request.output_mode == "files_with_matches" {
                    break;
                }
            }
            if request.output_mode == "count"
                && count > 0
                && !result.push(
                    json!({"path":path,"matching_lines":count}),
                    limit,
                    self.config.max_output_bytes,
                    &mut output_bytes,
                )
            {
                break;
            }
        }
        result.complete =
            result.reason.is_none() && result.skipped_io == 0 && result.skipped_large == 0;
        let mut value = serde_json::to_value(result).map_err(|e| e.to_string())?;
        value["root_id"] = json!(request.root_id);
        value["output_mode"] = json!(request.output_mode);
        Ok(value)
    }

    fn load_rules(
        &self,
        root: &Root,
        dir: &Path,
        rules: &mut Vec<Arc<Gitignore>>,
    ) -> Result<(), String> {
        for name in [".gitignore", ".ignore"] {
            let path = dir.join(name);
            let meta = match root.dir.symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("cannot read ignore rules: {e}")),
            };
            if is_link(&meta) || !meta.is_file() {
                continue;
            }
            let bytes = read_bounded(&root.dir, &path, self.config.max_file_bytes)
                .map_err(|e| format!("cannot read ignore rules: {e}"))?
                .ok_or("ignore file exceeds max_file_bytes")?;
            let text = std::str::from_utf8(&bytes).map_err(|_| "ignore file is not UTF-8")?;
            let mut builder = GitignoreBuilder::new(root.path.join(dir));
            for line in text.lines() {
                builder
                    .add_line(None, line)
                    .map_err(|e| format!("invalid ignore rule: {e}"))?;
            }
            rules.push(Arc::new(builder.build().map_err(|e| e.to_string())?));
        }
        Ok(())
    }
}

fn checked_relative(path: &str) -> Result<PathBuf, String> {
    // Reject Windows drive/ADS/UNC syntax on every platform too.
    if path.contains([':', '\\', '\0']) {
        return Err("path must use root-relative forward slashes".into());
    }
    let path = Path::new(path);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err("absolute paths and parent traversal are forbidden".into());
    }
    Ok(path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect())
}
fn is_link(meta: &cap_std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}
fn hidden(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
}
fn ignored(root: &Root, path: &Path, is_dir: bool, rules: &[Arc<Gitignore>]) -> bool {
    let full = root.path.join(path);
    rules
        .iter()
        .rev()
        .find_map(|r| {
            let m = r.matched_path_or_any_parents(&full, is_dir);
            if m.is_none() {
                None
            } else {
                Some(m.is_ignore())
            }
        })
        .unwrap_or(false)
}
fn read_bounded(dir: &Dir, path: &Path, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let file = dir.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() <= max).then_some(bytes))
}
fn clip(text: &str, max: usize) -> (&str, bool) {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], end < text.len())
}

#[derive(Default, Serialize)]
struct SearchResult {
    results: Vec<Value>,
    complete: bool,
    truncated: bool,
    reason: Option<String>,
    searched_files: usize,
    skipped_io: usize,
    skipped_large: usize,
    skipped_binary: usize,
    skipped_links: usize,
}
impl SearchResult {
    fn stop(&mut self, reason: &str) {
        self.truncated = true;
        self.reason = Some(reason.into());
    }
    fn push(&mut self, item: Value, limit: usize, max: usize, used: &mut usize) -> bool {
        if self.results.len() >= limit {
            self.stop("max_results");
            return false;
        }
        let size = item.to_string().len() + 1;
        // Reserve bounded room for envelope fields. Never cut serialized JSON mid-item.
        if used.saturating_add(size) > max.saturating_sub(768) {
            self.stop("max_output_bytes");
            return false;
        }
        *used += size;
        self.results.push(item);
        true
    }
}

#[derive(Default)]
struct Request {
    root_id: String,
    path: String,
    pattern: String,
    glob: String,
    literal: bool,
    ignore_case: bool,
    output_mode: String,
    limit: usize,
}

#[define_operation(
    name = "Grep", display_name = "搜索{root_id}/{path}，模式{pattern}，过滤{glob}，字面量{literal}，忽略大小写{ignore_case}，输出{output_mode}，上限{limit}",
    category = "Search", system_only,
    description = "Search UTF-8 text in host-authorized local roots. Omit root_id to list authorized IDs. Supply root_id, relative path (forward slashes) and pattern to search. Literal or Rust regex; modes content/files_with_matches/count. Hidden, ignored, binary and link files are skipped. Results report incompleteness. AI-only, unavailable in Workflow scripts.",
    params {
        root_id: "String@Authorized root ID; omit to list IDs.",
        path: "String@Relative file or directory, default root. No parent traversal.",
        pattern: "String@Nonempty search pattern.",
        glob: "String@Optional root-relative file glob, e.g. **/*.rs.",
        literal: "bool@Match literal text instead of regex. Default true.",
        ignore_case: "bool@Case insensitive, default false.",
        output_mode: "String@content (default), files_with_matches, or count (matching lines).",
        limit: "u64@Positive result limit, capped by host configuration."
    }, destructive = false, readonly = true, idempotent = true, open_world = false
)]
pub struct GrepSystem;

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("orbit-grep-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, path: &str, text: impl AsRef<[u8]>) {
            let target = self.0.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text).unwrap();
        }
        fn service(&self, mut config: GrepConfig) -> GrepService {
            config.roots = vec![GrepRoot {
                id: "project".into(),
                path: self.0.clone(),
            }];
            GrepService::new(config, None).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn request() -> Request {
        Request {
            root_id: "project".into(),
            pattern: "needle".into(),
            literal: true,
            output_mode: "content".into(),
            limit: 100,
            ..Default::default()
        }
    }
    fn search(service: &GrepService, request: Request) -> Value {
        service.search(request, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn grep_content_modes_ignore_and_unicode() {
        let f = Fixture::new();
        f.write("src/a.rs", "first\nneedle 中文\nneedle two\n");
        f.write("src/ignored.rs", "needle");
        f.write("src/.ignore", "ignored.rs\n");
        f.write("other.txt", "needle");
        f.write(".gitignore", "other.txt\n");
        f.write(".secret", "needle");
        f.write("binary", b"needle\0");
        let service = f.service(GrepConfig::default());
        let result = search(&service, request());
        assert_eq!(result["results"].as_array().unwrap().len(), 2);
        assert_eq!(result["results"][0]["line"], 2);
        assert_eq!(result["results"][0]["text"], "needle 中文");
        assert_eq!(result["complete"], true);
        assert_eq!(result["skipped_binary"], 1);
        let mut req = request();
        req.output_mode = "count".into();
        assert_eq!(search(&service, req)["results"][0]["matching_lines"], 2);
        let mut req = request();
        req.output_mode = "files_with_matches".into();
        assert_eq!(
            search(&service, req)["results"].as_array().unwrap().len(),
            1
        );
        let mut req = request();
        req.path = "src".into();
        req.glob = "**/*.rs".into();
        assert_eq!(
            search(&service, req)["results"].as_array().unwrap().len(),
            2
        );
        let mut req = request();
        req.pattern = "NEEDLE.*".into();
        req.literal = false;
        req.ignore_case = true;
        assert_eq!(
            search(&service, req)["results"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn grep_rejects_bad_inputs_and_never_grants_authority_from_path() {
        let f = Fixture::new();
        f.write("a", "needle");
        let service = f.service(GrepConfig::default());
        for path in [
            "../outside",
            "/outside",
            "C:/outside",
            "a/../../outside",
            "\\\\server\\share",
            "a:stream",
        ] {
            let mut req = request();
            req.path = path.into();
            assert!(
                service.search(req, &AtomicBool::new(false)).is_err(),
                "{path}"
            );
        }
        let mut req = request();
        req.root_id = "unknown".into();
        assert!(service
            .search(req, &AtomicBool::new(false))
            .unwrap_err()
            .contains("authorized"));
        let mut req = request();
        req.pattern = "[".into();
        req.literal = false;
        assert!(service
            .search(req, &AtomicBool::new(false))
            .unwrap_err()
            .contains("pattern"));
        let mut req = request();
        req.glob = "[".into();
        assert!(service
            .search(req, &AtomicBool::new(false))
            .unwrap_err()
            .contains("glob"));
        let mut req = request();
        req.pattern = "missing".into();
        let result = search(&service, req);
        assert_eq!(result["results"], json!([]));
        assert_eq!(result["complete"], true);
        let list = search(&service, Request::default());
        assert_eq!(list["roots"], json!(["project"]));
        assert!(!list
            .to_string()
            .contains(&f.0.to_string_lossy().to_string()));
    }

    #[test]
    fn grep_limits_and_cancellation_are_explicit() {
        let f = Fixture::new();
        f.write("a", "needle 中文\nneedle two\nneedle three");
        let service = f.service(GrepConfig {
            max_results: 1,
            max_line_bytes: 8,
            ..Default::default()
        });
        let result = search(&service, request());
        assert_eq!(result["results"].as_array().unwrap().len(), 1);
        assert_eq!(result["reason"], "max_results");
        assert_eq!(result["complete"], false);
        assert_eq!(result["results"][0]["line_truncated"], true);
        let result = service.search(request(), &AtomicBool::new(true)).unwrap();
        assert_eq!(result["complete"], false);
        let service = f.service(GrepConfig {
            max_file_bytes: 10,
            ..Default::default()
        });
        let result = search(&service, request());
        assert_eq!(result["skipped_large"], 1);
        assert_eq!(result["complete"], false);
        f.write("a", "needle".repeat(400));
        let service = f.service(GrepConfig {
            max_output_bytes: 1024,
            ..Default::default()
        });
        let result = search(&service, request());
        assert_eq!(result["reason"], "max_output_bytes");
        assert!(result.to_string().len() <= 1024);
    }

    #[test]
    fn grep_is_ai_only_and_has_strict_schema() {
        let factory = inventory::iter::<corework::ai_system::AISystemFactory>
            .into_iter()
            .find(|f| f.metadata.name == "Grep")
            .unwrap();
        assert!(!factory.metadata.workflow_enabled);
        assert!(factory.metadata.readonly);
        assert!(corework::workflow::registry::NodeRegistry::get("Grep").is_none());
        crate::tool_schema::definitions_for_active_tools(&["Grep".into()], true, true).unwrap();
    }

    #[tokio::test]
    async fn grep_without_host_authority_fails_closed() {
        let ctx = Context::new(
            Arc::new(corework::cache::InMemoryCache::new()),
            Arc::new(corework::event::InMemoryEventBus::new()),
            Arc::new(corework::monitoring::NoopTelemetry),
        );
        let result = GrepSystem
            .execute(AIInput::from_args(Default::default()), &ctx)
            .await
            .unwrap();
        assert_eq!(result.error_code, 403);
    }

    #[cfg(unix)]
    #[test]
    fn grep_does_not_follow_symlinks() {
        let f = Fixture::new();
        let outside = Fixture::new();
        outside.write("secret", "needle");
        std::os::unix::fs::symlink(&outside.0, f.0.join("link")).unwrap();
        let service = f.service(GrepConfig::default());
        assert_eq!(search(&service, request())["results"], json!([]));
        let mut req = request();
        req.path = "link/secret".into();
        assert!(service.search(req, &AtomicBool::new(false)).is_err());
        // Capability open itself rejects an escape even if traversal raced with a link replacement.
        assert!(service.roots["project"].dir.open("link/secret").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn grep_rejects_windows_junctions() {
        let f = Fixture::new();
        let outside = Fixture::new();
        outside.write("secret", "needle");
        let link = f.0.join("junction");
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let service = f.service(GrepConfig::default());
        let result = search(&service, request());
        let mut req = request();
        req.path = "junction/secret".into();
        let rejected = service.search(req, &AtomicBool::new(false)).is_err();
        let capability_rejected = service.roots["project"]
            .dir
            .open("junction/secret")
            .is_err();
        std::fs::remove_dir(&link).unwrap();
        assert_eq!(result["results"], json!([]));
        assert_eq!(result["skipped_links"], 1);
        assert!(rejected);
        assert!(capability_rejected);
    }

    #[tokio::test]
    async fn grep_executes_through_shared_authority_and_limits_concurrency() {
        use corework::execution_unit::{ExecutionUnit, UnitType};
        let f = Fixture::new();
        f.write("file", "needle 中文");
        let service = Arc::new(f.service(GrepConfig {
            max_concurrent_searches: 1,
            ..Default::default()
        }));
        let unit = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            corework::world::FrameworkState::initialize().unwrap(),
            format!("grep-{}", uuid::Uuid::new_v4()),
        ));
        unit.attach_shared_component(service.clone()).unwrap();
        let input = || {
            AIInput::from_args(std::collections::HashMap::from([
                ("root_id".into(), "project".into()),
                ("pattern".into(), "needle".into()),
            ]))
        };
        let ctx = unit.create_context();
        let result = GrepSystem.execute(input(), &ctx).await.unwrap();
        assert_eq!(result.error_code, 0, "{}", result.to_ai);
        assert_eq!(
            serde_json::from_str::<Value>(&result.to_ai).unwrap(),
            result.result
        );
        assert_eq!(result.result["results"][0]["text"], "needle 中文");
        let _permit = service.permits.clone().acquire_owned().await.unwrap();
        assert_eq!(
            GrepSystem.execute(input(), &ctx).await.unwrap().error_code,
            429
        );
    }
}

#[async_trait]
impl SystemOperation for GrepSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;
    fn name(&self) -> &str {
        "Grep"
    }
    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let service = match ctx.resolve_shared_component::<GrepService>() {
            Ok(v) => v,
            Err(_) => {
                return Ok(AIOutput::error(
                    403,
                    "Host has not authorized local search roots",
                ))
            }
        };
        let boolean = |name, default| -> Result<bool, String> {
            args.get(name)
                .map(|v| v.parse().map_err(|_| format!("{name} must be boolean")))
                .unwrap_or(Ok(default))
        };
        let request = (|| -> Result<Request, String> {
            Ok(Request {
                root_id: args.get("root_id").unwrap_or("").into(),
                path: args.get("path").unwrap_or("").into(),
                pattern: args.get("pattern").unwrap_or("").into(),
                glob: args.get("glob").unwrap_or("").into(),
                literal: boolean("literal", true)?,
                ignore_case: boolean("ignore_case", false)?,
                output_mode: args.get("output_mode").unwrap_or("content").into(),
                limit: args
                    .get("limit")
                    .map(|v| {
                        v.parse()
                            .map_err(|_| "limit must be a positive integer".to_string())
                    })
                    .unwrap_or(Ok(service.config.max_results))?,
            })
        })();
        let request = match request {
            Ok(v) => v,
            Err(e) => return Ok(AIOutput::error(400, e)),
        };
        let permit = match service.permits.clone().try_acquire_owned() {
            Ok(v) => v,
            Err(_) => {
                return Ok(AIOutput::error(
                    429,
                    "Local search concurrency limit reached; retry later",
                ))
            }
        };
        let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
        let flag = cancel.0.clone();
        let timeout = Duration::from_millis(service.config.timeout_ms);
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            service.search(request, &flag)
        });
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(Ok(value))) => {
                let text = value.to_string();
                Ok(AIOutput::success(value, text))
            }
            Ok(Ok(Err(e))) => Ok(AIOutput::error(400, e)),
            Ok(Err(e)) => Ok(AIOutput::error(500, format!("Search worker failed: {e}"))),
            Err(_) => Ok(AIOutput::error(
                408,
                "Search timed out; results are incomplete. Narrow the search.",
            )),
        }
    }
}
