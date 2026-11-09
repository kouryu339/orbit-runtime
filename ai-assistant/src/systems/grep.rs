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
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrepRoot {
    pub id: String,
    pub path: PathBuf,
    /// Host-only directory containing per-conversation grants.
    #[serde(default)]
    pub conversation_grants: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GrepConfig {
    pub roots: Vec<GrepRoot>,
    /// Require an explicit cluster ceiling and host API grant for every conversation.
    pub require_conversation_grants: bool,
    pub timeout_ms: u64,
    pub max_results: usize,
    pub max_output_bytes: usize,
    pub max_file_bytes: usize,
    pub max_decoded_bytes: usize,
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
            require_conversation_grants: false,
            timeout_ms: 10_000,
            max_results: 200,
            max_output_bytes: 65_536,
            max_file_bytes: 2 * 1024 * 1024,
            max_decoded_bytes: 16 * 1024 * 1024,
            max_pattern_bytes: 16 * 1024,
            max_regex_bytes: 8 * 1024 * 1024,
            max_line_bytes: 4096,
            max_entries: 100_000,
            max_concurrent_searches: 2,
        }
    }
}

#[derive(Debug, Clone)]
struct Root {
    dir: Arc<Dir>,
    path: PathBuf,
    grants: Option<Arc<Dir>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadAccessGrant {
    pub root_id: String,
    pub subpath: String,
}

#[derive(Debug)]
struct ConversationAccess {
    conversation_id: String,
    required: bool,
    /// None selects legacy file grants only when not required. Some(empty) denies all.
    roots: RwLock<Option<BTreeMap<String, Root>>>,
}

#[derive(Debug)]
pub struct GrepService {
    pub(super) config: GrepConfig,
    roots: BTreeMap<String, Root>,
    pub(super) permits: Arc<tokio::sync::Semaphore>,
    access: Option<Arc<ConversationAccess>>,
    pub decoders: Arc<super::file_decode::DecoderRegistry>,
    identity: Option<String>,
}
impl GrepService {
    /// Called by the host at registration. Open directory capabilities pin authority.
    pub fn new(mut config: GrepConfig, base: Option<&Path>) -> Result<Self, String> {
        if config.timeout_ms == 0
            || config.max_results == 0
            || config.max_output_bytes < 1024
            || config.max_file_bytes == 0
            || config.max_decoded_bytes == 0
            || config.max_decoded_bytes > 64 * 1024 * 1024
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
            let grants = root
                .conversation_grants
                .as_ref()
                .map(|path| {
                    let path = if path.is_absolute() {
                        path.clone()
                    } else {
                        base.ok_or("relative grep grants require a resource base directory")?
                            .join(path)
                    };
                    Dir::open_ambient_dir(&path, cap_std::ambient_authority())
                        .map(Arc::new)
                        .map_err(|error| format!("grep grants '{}': {error}", root.id))
                })
                .transpose()?;
            roots.insert(
                root.id.clone(),
                Root {
                    dir: Arc::new(dir),
                    path: root.path.clone(),
                    grants,
                },
            );
        }
        Ok(Self {
            permits: Arc::new(tokio::sync::Semaphore::new(config.max_concurrent_searches)),
            config,
            roots,
            access: None,
            decoders: Arc::new(super::file_decode::DecoderRegistry::default()),
            identity: None,
        })
    }

    /// Bind the trusted conversation and its cluster ceiling before any tools run.
    pub fn bind_conversation(
        &self,
        conversation_id: &str,
        root_ids: Option<&[String]>,
    ) -> Result<Self, String> {
        self.decoders.freeze();
        let mut roots = BTreeMap::new();
        if let Some(ids) = root_ids {
            for id in ids {
                let root = self
                    .roots
                    .get(id)
                    .ok_or_else(|| format!("unknown Grep root: {id}"))?;
                if roots.insert(id.clone(), root.clone()).is_some() {
                    return Err(format!("duplicate Grep root: {id}"));
                }
            }
        } else if !self.config.require_conversation_grants {
            roots = self.roots.clone();
        }
        Ok(Self {
            config: self.config.clone(),
            roots,
            permits: self.permits.clone(),
            decoders: self.decoders.clone(),
            identity: Some(conversation_id.to_owned()),
            access: Some(Arc::new(ConversationAccess {
                conversation_id: conversation_id.to_string(),
                required: self.config.require_conversation_grants || root_ids.is_some(),
                roots: RwLock::new(None),
            })),
        })
    }

    /// Host-only atomic replacement. Validate the whole candidate before publishing.
    /// Holding the write lock also waits for reads admitted under the previous grant.
    pub fn set_read_access(
        &self,
        conversation_id: &str,
        grants: &[ReadAccessGrant],
    ) -> Result<(), String> {
        let access = self
            .access
            .as_ref()
            .filter(|access| access.conversation_id == conversation_id)
            .ok_or("Grep service is not bound to this conversation")?;
        let mut candidate = BTreeMap::new();
        for grant in grants {
            let root = self.roots.get(&grant.root_id).ok_or_else(|| {
                format!(
                    "root_id is outside the cluster's Grep scope: {}",
                    grant.root_id
                )
            })?;
            if candidate.contains_key(&grant.root_id) {
                return Err(format!(
                    "one subpath per root_id is supported: {}",
                    grant.root_id
                ));
            }
            let relative = checked_relative(&grant.subpath)?;
            let mut dir = root.dir.try_clone().map_err(|e| e.to_string())?;
            let mut ancestor = PathBuf::new();
            for component in relative.components() {
                ancestor.push(component);
                let metadata = dir
                    .symlink_metadata(component.as_os_str())
                    .map_err(|e| e.to_string())?;
                if is_link(&metadata) || !metadata.is_dir() || hidden(&ancestor) {
                    return Err("grant subpath must be an existing directory without hidden components or links".into());
                }
                // Open each component without following links, so replacing a
                // directory between inspection and open cannot broaden a grant.
                let parent = dir.into_std_file();
                let child = cap_primitives::fs::open_dir_nofollow(
                    &parent,
                    Path::new(component.as_os_str()),
                )
                .map_err(|e| e.to_string())?;
                dir = Dir::from_std_file(child);
            }
            candidate.insert(
                grant.root_id.clone(),
                Root {
                    dir: Arc::new(dir),
                    path: root.path.join(relative),
                    grants: None,
                },
            );
        }
        *access
            .roots
            .write()
            .map_err(|_| "Grep authorization lock poisoned")? = Some(candidate);
        tracing::info!(
            conversation_id,
            grant_count = grants.len(),
            "Grep read access replaced"
        );
        Ok(())
    }

    pub fn revoke_read_access(&self, conversation_id: &str) -> Result<(), String> {
        self.set_read_access(conversation_id, &[])
    }

    /// Recovery invalidates host API grants instead of replaying them from history.
    pub fn invalidate_restored_read_access(&self) -> Result<(), String> {
        if let Some(access) = &self.access {
            let mut roots = access
                .roots
                .write()
                .map_err(|_| "Grep authorization lock poisoned")?;
            if access.required || roots.is_some() {
                *roots = Some(BTreeMap::new());
            }
        }
        Ok(())
    }

    /// Keep the grant read lock through the complete blocking read, including paging.
    pub(super) fn with_conversation<T>(
        &self,
        conversation_id: Option<&str>,
        read: impl FnOnce(&Self) -> Result<T, String>,
    ) -> Result<T, String> {
        if let Some(access) = &self.access {
            if conversation_id != Some(access.conversation_id.as_str()) {
                return Err("read_access_denied: conversation identity mismatch".into());
            }
            let guard = access
                .roots
                .read()
                .map_err(|_| "Grep authorization lock poisoned")?;
            if let Some(roots) = guard.as_ref() {
                let scoped = Self {
                    config: self.config.clone(),
                    roots: roots.clone(),
                    permits: self.permits.clone(),
                    access: None,
                    decoders: self.decoders.clone(),
                    identity: self.identity.clone(),
                };
                return read(&scoped);
            }
            if access.required {
                return Err(
                    "read_access_required: host must grant this conversation read access".into(),
                );
            }
            return read(&self.for_conversation(conversation_id));
        }
        if self.config.require_conversation_grants {
            return Err("read_access_required: trusted conversation binding is missing".into());
        }
        read(&self.for_conversation(conversation_id))
    }

    /// Derive directory capabilities from the trusted runtime conversation identity.
    /// Model arguments never participate in grant resolution.
    fn for_conversation(&self, conversation_id: Option<&str>) -> Self {
        fn component(value: &str) -> bool {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        }
        let grant_path = conversation_id
            .filter(|id| component(id))
            .map(|id| format!("{id}.txt"));
        let exclusive = grant_path.as_ref().is_some_and(|path| {
            self.roots.values().filter_map(|root| root.grants.as_ref()).any(|grants| {
                !matches!(grants.symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
            })
        });
        let roots = self
            .roots
            .iter()
            .filter_map(|(id, root)| {
                let Some(grants) = &root.grants else {
                    return (!exclusive).then(|| {
                        (
                            id.clone(),
                            Root {
                                dir: root.dir.clone(),
                                path: root.path.clone(),
                                grants: None,
                            },
                        )
                    });
                };
                let grant_path = grant_path.as_ref()?;
                let metadata = grants.symlink_metadata(grant_path).ok()?;
                if is_link(&metadata) || !metadata.is_file() {
                    return None;
                }
                let bytes = read_bounded(grants, Path::new(grant_path), 256).ok()??;
                let project = std::str::from_utf8(&bytes).ok()?.trim();
                if !component(project) {
                    return None;
                }
                let metadata = root.dir.symlink_metadata(project).ok()?;
                if is_link(&metadata) || !metadata.is_dir() {
                    return None;
                }
                let dir = Arc::new(root.dir.open_dir(project).ok()?);
                Some((
                    id.clone(),
                    Root {
                        dir,
                        path: root.path.join(project),
                        grants: None,
                    },
                ))
            })
            .collect();
        Self {
            config: self.config.clone(),
            roots,
            permits: self.permits.clone(),
            access: None,
            decoders: self.decoders.clone(),
            identity: conversation_id.map(str::to_owned),
        }
    }

    pub(super) fn decode_file(
        &self,
        path: &Path,
        bytes: Vec<u8>,
        representation: &str,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<super::file_decode::Decoded, String> {
        self.decoders.decode(
            path,
            bytes,
            representation,
            self.identity.as_deref(),
            cancel,
            deadline,
            self.config.max_decoded_bytes,
        )
    }

    pub(super) fn authorized_ids(&self) -> Vec<&str> {
        self.roots.keys().map(String::as_str).collect()
    }

    /// Single-file reads share Grep's root capabilities and exclusion policy.
    pub(super) fn read_authorized_file(
        &self,
        root_id: &str,
        path: &str,
    ) -> Result<Vec<u8>, String> {
        let root = self.roots.get(root_id).ok_or("root_id is not authorized")?;
        let relative = checked_relative(path)?;
        if relative.as_os_str().is_empty() {
            return Err("path must name one JSON file".into());
        }
        let mut ancestor = PathBuf::new();
        let mut rules = Vec::new();
        self.load_rules(root, Path::new(""), &mut rules)?;
        for component in relative.components() {
            ancestor.push(component);
            let meta = root
                .dir
                .symlink_metadata(&ancestor)
                .map_err(|e| format!("cannot inspect JSON path: {e}"))?;
            if is_link(&meta)
                || hidden(&ancestor)
                || ignored(root, &ancestor, meta.is_dir(), &rules)
            {
                return Err(
                    "JSON path is hidden, ignored, or contains a link/reparse point".into(),
                );
            }
            if ancestor != relative && meta.is_dir() {
                self.load_rules(root, &ancestor, &mut rules)?;
            }
        }
        let mut file = root
            .dir
            .open(&relative)
            .map_err(|e| format!("cannot open JSON: {e}"))?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() {
            return Err("JSON path is not a regular file".into());
        }
        if before.len() > self.config.max_file_bytes as u64 {
            return Err("JSON exceeds host max_file_bytes".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(self.config.max_file_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if bytes.len() > self.config.max_file_bytes {
            return Err("JSON exceeds host max_file_bytes".into());
        }
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err("JSON changed during read; retry".into());
        }
        Ok(bytes)
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
            let decoded = self.decode_file(
                &path,
                bytes,
                "text",
                cancel,
                start + Duration::from_millis(self.config.timeout_ms),
            )?;
            let bytes = &decoded.bytes;
            let text = match std::str::from_utf8(bytes) {
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
                if index + 1 < request.start_line || !regex.is_match(line) {
                    continue;
                }
                count += 1;
                if request.output_mode == "count" {
                    continue;
                }
                let mut item = if request.output_mode == "files_with_matches" {
                    json!({"path":path})
                } else {
                    let (snippet, cut) = clip(line, self.config.max_line_bytes);
                    json!({"path":path,"line":index+1,"text":snippet,"line_truncated":cut})
                };
                if let Some(decoder) = &decoded.decoder {
                    item["decoder"] = json!(decoder);
                    item["revision"] = json!(decoded.revision);
                    if let Some(location) = decoded.locations.iter().find(|loc| {
                        loc["start_line"]
                            .as_u64()
                            .is_some_and(|v| v <= (index + 1) as u64)
                            && loc["end_line"]
                                .as_u64()
                                .is_some_and(|v| v >= (index + 1) as u64)
                    }) {
                        item["location"] = location["location"].clone();
                    }
                }
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
    start_line: usize,
}

#[define_operation(
    name = "Grep", display_name = "搜索{root_id}/{path}，模式{pattern}，过滤{glob}，字面量{literal}，忽略大小写{ignore_case}，输出{output_mode}，上限{limit}，起始行{start_line}",
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
        limit: "u64@Positive result limit, capped by host configuration.",
        start_line: "u64@First 1-based line to search in the selected file; use last result line plus one to continue."
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
                conversation_grants: None,
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
    fn a_conversation_grant_is_exclusive_and_paginates_its_own_files() {
        let f = Fixture::new();
        f.write("logs/a/turn.txt", "needle first\nneedle second\n");
        f.write("logs/b/turn.txt", "needle other project");
        f.write("global/log.txt", "needle global");
        f.write("grants/own.txt", "a");
        f.write("grants/revoked.txt", "");
        let service = GrepService::new(
            GrepConfig {
                roots: vec![
                    GrepRoot {
                        id: "project".into(),
                        path: f.0.join("logs"),
                        conversation_grants: Some(f.0.join("grants")),
                    },
                    GrepRoot {
                        id: "global".into(),
                        path: f.0.join("global"),
                        conversation_grants: None,
                    },
                ],
                ..GrepConfig::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(
            service.for_conversation(Some("unknown")).authorized_ids(),
            ["global"]
        );
        assert!(service
            .for_conversation(Some("revoked"))
            .authorized_ids()
            .is_empty());
        let own = service.for_conversation(Some("own"));
        assert_eq!(own.authorized_ids(), ["project"]);
        assert!(own.read_authorized_file("global", "log.txt").is_err());
        assert!(own
            .read_authorized_file("project", "../b/turn.txt")
            .is_err());
        let mut page = request();
        page.path = "turn.txt".into();
        page.start_line = 2;
        let result = search(&own, page);
        assert_eq!(result["results"].as_array().unwrap().len(), 1);
        assert_eq!(result["results"][0]["text"], "needle second");
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
    fn host_read_access_is_conversation_scoped_atomic_and_bounded_by_cluster() {
        let f = Fixture::new();
        f.write("a/nested/data.json", r#"{"project":"A"}"#);
        f.write("b/data.json", r#"{"project":"B"}"#);
        let global = f.service(GrepConfig {
            require_conversation_grants: true,
            ..Default::default()
        });
        let a = global
            .bind_conversation("conv-a", Some(&["project".into()]))
            .unwrap();
        let b = global
            .bind_conversation("conv-b", Some(&["project".into()]))
            .unwrap();
        let missing_cluster = global.bind_conversation("no-cluster", None).unwrap();
        let read = |service: &GrepService, id: &str| {
            service.with_conversation(Some(id), |scoped| {
                scoped.read_authorized_file("project", "data.json")
            })
        };
        assert!(read(&a, "conv-a")
            .unwrap_err()
            .contains("read_access_required"));
        let grants = [ReadAccessGrant {
            root_id: "project".into(),
            subpath: "a/nested".into(),
        }];
        assert!(missing_cluster
            .set_read_access("no-cluster", &grants)
            .is_err());
        a.set_read_access("conv-a", &grants).unwrap();
        assert_eq!(read(&a, "conv-a").unwrap(), br#"{"project":"A"}"#);
        assert!(read(&a, "conv-b")
            .unwrap_err()
            .contains("identity mismatch"));
        assert!(read(&b, "conv-b").is_err());
        assert!(a
            .set_read_access(
                "conv-a",
                &[
                    ReadAccessGrant {
                        root_id: "project".into(),
                        subpath: "b".into()
                    },
                    ReadAccessGrant {
                        root_id: "missing".into(),
                        subpath: "".into()
                    },
                ]
            )
            .is_err());
        for path in ["../b", "/b", "C:/b", r"a\nested", "a/missing"] {
            assert!(a
                .set_read_access(
                    "conv-a",
                    &[ReadAccessGrant {
                        root_id: "project".into(),
                        subpath: path.into(),
                    }]
                )
                .is_err());
        }
        assert_eq!(read(&a, "conv-a").unwrap(), br#"{"project":"A"}"#);
        a.revoke_read_access("conv-a").unwrap();
        assert!(read(&a, "conv-a").is_err());
        let fresh = global
            .bind_conversation("conv-a", Some(&["project".into()]))
            .unwrap();
        assert!(read(&fresh, "conv-a")
            .unwrap_err()
            .contains("read_access_required"));
    }

    #[test]
    fn revoke_waits_for_admitted_reads_and_prevents_new_reads() {
        let f = Fixture::new();
        f.write("a/data.json", "{}");
        let service = Arc::new(
            f.service(GrepConfig::default())
                .bind_conversation("conv", Some(&["project".into()]))
                .unwrap(),
        );
        service
            .set_read_access(
                "conv",
                &[ReadAccessGrant {
                    root_id: "project".into(),
                    subpath: "a".into(),
                }],
            )
            .unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let reader = service.clone();
        let reading = std::thread::spawn(move || {
            reader.with_conversation(Some("conv"), |scoped| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                scoped.read_authorized_file("project", "data.json")
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (revoked_tx, revoked_rx) = std::sync::mpsc::channel();
        let writer = service.clone();
        let revoking = std::thread::spawn(move || {
            writer.revoke_read_access("conv").unwrap();
            revoked_tx.send(()).unwrap();
        });
        assert!(revoked_rx.recv_timeout(Duration::from_millis(30)).is_err());
        release_tx.send(()).unwrap();
        assert_eq!(reading.join().unwrap().unwrap(), b"{}");
        revoked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        revoking.join().unwrap();
        assert!(service
            .with_conversation(Some("conv"), |scoped| {
                scoped.read_authorized_file("project", "data.json")
            })
            .is_err());
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
                start_line: args
                    .get("start_line")
                    .unwrap_or("1")
                    .parse()
                    .map_err(|_| "start_line must be a line number")?,
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
        let conversation_id = ctx.conversation_id.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            service.with_conversation(conversation_id.as_deref(), |scoped| {
                scoped.search(request, &flag)
            })
        });
        match corework::platform::time::timeout(timeout, task).await {
            Ok(Ok(Ok(value))) => {
                let text = value.to_string();
                Ok(AIOutput::success(value, text))
            }
            Ok(Ok(Err(e))) => Ok(AIOutput::error(
                if e.starts_with("read_access_") {
                    403
                } else {
                    400
                },
                e,
            )),
            Ok(Err(e)) => Ok(AIOutput::error(500, format!("Search worker failed: {e}"))),
            Err(_) => Ok(AIOutput::error(
                408,
                "Search timed out; results are incomplete. Narrow the search.",
            )),
        }
    }
}
