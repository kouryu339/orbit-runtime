//! Runtime-owned Skill source transactions. Immutable source objects are staged
//! first; a single atomic manifest replacement is the durable commit point.
use super::*;
use sha2::{Digest, Sha256};

const SCHEMA: &str = "orbit.runtime.skill-store/v1";
#[cfg(test)]
thread_local! { pub(super) static FAIL_POINT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) }; }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredSkill {
    source: String,
    version: String,
    object: String,
    asset_base: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManagedState {
    schema: String,
    pub revision: u64,
    skills: BTreeMap<String, StoredSkill>,
    pub clusters: BTreeMap<String, RuntimeAgentCluster>,
}
impl Default for ManagedState {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            revision: 0,
            skills: BTreeMap::new(),
            clusters: BTreeMap::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    name: String,
    kind: String,
    source: String,
    // null means create; a digest means replace that exact accepted version.
    expected_version: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClusterChange {
    registration: Value,
    expected_version: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Transaction {
    expected_store_revision: u64,
    skills: Vec<Candidate>,
    cluster: Option<ClusterChange>,
}

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidConfig(message.into())
}
fn io(error: std::io::Error) -> RuntimeError {
    RuntimeError::Internal(format!("Skill store IO: {error}"))
}
fn source_object(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

// No symlink/junction inside the configured store. The root is a trusted host
// configuration, never a caller-supplied transaction path.
fn plain_path(path: &Path) -> Result<(), RuntimeError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(invalid("Skill store cannot contain symlinks"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(invalid("Skill store cannot contain reparse points"));
            }
        }
    }
    Ok(())
}

pub(super) fn read_managed_state(root: &Path) -> Result<ManagedState, RuntimeError> {
    plain_path(root)?;
    let path = root.join("catalog.json");
    plain_path(&path)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ManagedState::default())
        }
        Err(error) => return Err(io(error)),
    };
    let state: ManagedState = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("Skill store is invalid: {error}")))?;
    if state.schema != SCHEMA || state.revision == 0 {
        return Err(invalid("Invalid Skill store header"));
    }
    for (name, skill) in &state.skills {
        if !valid_name(name) || skill.object != source_object(&skill.source) {
            return Err(invalid("Skill store object identity mismatch"));
        }
        for path in [
            root.join("objects"),
            root.join("objects").join(&skill.object),
            object_path(root, &skill.object),
        ] {
            plain_path(&path)?;
        }
        if fs::read(object_path(root, &skill.object)).map_err(io)? != skill.source.as_bytes() {
            return Err(invalid(format!(
                "Skill store source integrity failure: {name}"
            )));
        }
    }
    Ok(state)
}

fn object_path(root: &Path, object: &str) -> PathBuf {
    root.join("objects").join(object).join("SKILL.md")
}

pub(super) fn restore_managed_skills(
    root: &Path,
    catalog: &mut SkillManager,
) -> Result<ManagedState, RuntimeError> {
    let state = read_managed_state(root)?;
    for (name, saved) in &state.skills {
        let mut skill = ai_assistant::skills::loader::parse_skill_content(&saved.source)
            .map_err(|error| invalid(error.to_string()))?;
        if skill.metadata.name != *name
            || skill.metadata.system_layer
            || !matches!(skill.metadata.kind.as_deref(), Some("role" | "capability"))
            || skill.instructions.trim().is_empty()
        {
            return Err(invalid("Invalid managed Skill identity/kind/body"));
        }
        if catalog.get(name).is_some_and(|existing| {
            existing.metadata.system_layer || existing.metadata.kind != skill.metadata.kind
        }) {
            return Err(invalid(
                "Managed Skill conflicts with a system Skill or existing kind",
            ));
        }
        skill.base_path = saved.asset_base.clone().or_else(|| {
            object_path(root, &saved.object)
                .parent()
                .map(Path::to_path_buf)
        });
        // Verify the version without depending on filesystem locations.
        if skill_version(&skill)? != saved.version {
            return Err(invalid("Skill content version integrity failure"));
        }
        catalog.stage_candidate(skill);
    }
    catalog.bind_managed_root(root.to_path_buf());
    Ok(state)
}

fn skill_version(skill: &ai_assistant::Skill) -> Result<String, RuntimeError> {
    let bytes = serde_json::to_vec(&(&skill.metadata, &skill.instructions))
        .map_err(|error| invalid(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

// Flush a temporary file and replace the manifest without a missing-file window.
fn atomic_manifest(path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let outcome = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            extern "system" {
                fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
            }
            let from: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 1 | 8) } == 0 {
                return Err(io(std::io::Error::last_os_error()));
            }
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temporary, path).map_err(io)?;
        }
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(temporary);
    }
    outcome
}

fn persist(root: &Path, state: &ManagedState) -> Result<(), RuntimeError> {
    for saved in state.skills.values() {
        let path = object_path(root, &saved.object);
        let directory = path.parent().unwrap();
        plain_path(&root.join("objects"))?;
        plain_path(directory)?;
        fs::create_dir_all(directory).map_err(io)?;
        plain_path(&path)?;
        if path.exists() {
            if fs::read(&path).map_err(io)? != saved.source.as_bytes() {
                return Err(invalid("Immutable Skill object was changed"));
            }
        } else {
            atomic_manifest(&path, saved.source.as_bytes())?;
        }
        #[cfg(unix)]
        {
            fs::File::open(directory)
                .map_err(io)?
                .sync_all()
                .map_err(io)?;
        }
    }
    #[cfg(unix)]
    {
        if root.join("objects").exists() {
            fs::File::open(root.join("objects"))
                .map_err(io)?
                .sync_all()
                .map_err(io)?;
        }
        fs::File::open(root).map_err(io)?.sync_all().map_err(io)?;
    }
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| invalid(error.to_string()))?;
    #[cfg(test)]
    if FAIL_POINT.with(|point| {
        if point.get() == 1 {
            point.set(0);
            true
        } else {
            false
        }
    }) {
        return Err(invalid("Injected failure before manifest commit"));
    }
    atomic_manifest(&root.join("catalog.json"), &bytes)?;
    // No fallible operation follows the durable commit point.
    Ok(())
}

impl RuntimeFacade {
    fn managed_skill_root(&self) -> Result<PathBuf, RuntimeError> {
        self.registries
            .resources
            .as_ref()
            .map(|resources| resources.managed_skills_dir.clone())
            .ok_or_else(|| invalid("Skill transactions require registered resources"))
    }

    pub fn read_skill(&self, name: &str) -> Result<Value, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        if !valid_name(name) {
            return Err(invalid("Invalid Skill name"));
        }
        let root = self.managed_skill_root()?;
        let _admission = self
            .rt
            .block_on(ai_assistant::skills::usage::SKILL_UPDATE_GATE.read());
        let state = read_managed_state(&root)?;
        if state.revision != self.registries.skill_store_revision {
            return Err(RuntimeError::SkillStoreConflict {
                expected: self.registries.skill_store_revision,
                actual: state.revision,
            });
        }
        let exists = self.rt.block_on(async {
            let catalog = ai_assistant::skills::systems::SKILL_MANAGER
                .get()
                .ok_or_else(|| invalid("Skill catalog is not initialized"))?
                .read()
                .await;
            if catalog.managed_root() != Some(root.as_path()) {
                return Err(invalid("Skill catalog belongs to another resource root"));
            }
            Ok(catalog.get(name).is_some())
        })?;
        if !exists {
            return Ok(
                json!({"name":name,"exists":false,"source":null,"metadata":null,"version":null,"store_revision":state.revision,"managed":false}),
            );
        }
        let (skill, source) = self.rt.block_on(async {
            let catalog = ai_assistant::skills::systems::SKILL_MANAGER
                .get()
                .ok_or_else(|| invalid("Skill catalog is not initialized"))?;
            let mut catalog = catalog.write().await;
            if catalog.managed_root() != Some(root.as_path()) {
                return Err(invalid("Skill catalog belongs to another resource root"));
            }
            let skill = catalog
                .load(name)
                .await
                .map_err(|error| invalid(error.to_string()))?
                .clone();
            // Managed sources retain authored text; filesystem/embedded Skills
            // use the accepted cached metadata/body, never unaccepted disk edits.
            let source = if let Some(saved) = state.skills.get(name) {
                saved.source.clone()
            } else {
                format!(
                    "---\n{}\n---\n{}",
                    serde_json::to_string_pretty(&skill.metadata).unwrap(),
                    skill.instructions
                )
            };
            Ok::<_, RuntimeError>((skill, source))
        })?;
        Ok(
            json!({"name":name,"exists":true,"source":source,"metadata":skill.metadata,"version":skill_version(&skill)?,
            "store_revision":state.revision,"managed":state.skills.contains_key(name)}),
        )
    }

    pub fn skill_transaction(
        &mut self,
        payload: &Value,
        commit: bool,
    ) -> Result<Value, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        let request: Transaction = serde_json::from_value(payload.clone())
            .map_err(|error| invalid(format!("Skill transaction: {error}")))?;
        if (request.skills.is_empty() && request.cluster.is_none())
            || request.skills.len() > 64
            || request
                .skills
                .iter()
                .map(|skill| skill.source.len())
                .sum::<usize>()
                > 4 * 1024 * 1024
        {
            return Err(invalid(
                "Skill transaction requires 1..64 candidates, at most 4 MiB total",
            ));
        }
        let root = self.managed_skill_root()?;
        let _admission = self
            .rt
            .block_on(ai_assistant::skills::usage::SKILL_UPDATE_GATE.write());
        let mut state = read_managed_state(&root)?;
        if state.revision != request.expected_store_revision
            || state.revision != self.registries.skill_store_revision
        {
            return Err(RuntimeError::SkillStoreConflict {
                expected: request.expected_store_revision,
                actual: state.revision,
            });
        }
        let global = ai_assistant::skills::systems::SKILL_MANAGER
            .get()
            .ok_or_else(|| invalid("Skill catalog is not initialized"))?
            .clone();
        let mut staged = self.rt.block_on(async { global.read().await.clone() });
        if staged.managed_root() != Some(root.as_path()) {
            return Err(invalid("Skill catalog belongs to another resource root"));
        }
        let mut names = BTreeSet::new();
        let mut results = Vec::new();
        for candidate in request.skills {
            if !valid_name(&candidate.name)
                || !names.insert(candidate.name.clone())
                || candidate.source.len() > 1024 * 1024
            {
                return Err(invalid(
                    "Skill names must be unique safe identifiers; each source is at most 1 MiB",
                ));
            }
            let mut skill = ai_assistant::skills::loader::parse_skill_content(&candidate.source)
                .map_err(|error| invalid(error.to_string()))?;
            if skill.metadata.name != candidate.name
                || skill.metadata.kind.as_deref() != Some(candidate.kind.as_str())
                || !matches!(candidate.kind.as_str(), "role" | "capability")
                || skill.metadata.system_layer
                || skill.instructions.trim().is_empty()
                || skill.metadata.description.trim().is_empty()
            {
                return Err(invalid("Candidate name/kind must match non-system metadata with non-empty description and body"));
            }
            let asset_base = staged
                .get(&candidate.name)
                .and_then(|skill| skill.base_path.clone());
            if let Some(previous) = staged.get(&candidate.name) {
                if previous.metadata.system_layer
                    || previous.base_path.is_none()
                    || previous.metadata.kind != skill.metadata.kind
                {
                    return Err(invalid(
                        "Cannot replace embedded/system Skill or change kind",
                    ));
                }
                let version = self
                    .rt
                    .block_on(staged.content_versions(&[candidate.name.clone()]))
                    .map_err(|error| invalid(error.to_string()))?
                    .remove(&candidate.name)
                    .unwrap();
                if candidate.expected_version.as_deref() != Some(version.as_str()) {
                    return Err(RuntimeError::SkillVersionConflict {
                        name: candidate.name,
                        expected: candidate.expected_version,
                        actual: Some(version),
                    });
                }
            } else if candidate.expected_version.is_some() {
                return Err(RuntimeError::SkillVersionConflict {
                    name: candidate.name,
                    expected: candidate.expected_version,
                    actual: None,
                });
            }
            self.validate_candidate_references(&skill)?;
            let version = skill_version(&skill)?;
            let object = source_object(&candidate.source);
            skill.base_path = asset_base
                .clone()
                .or_else(|| object_path(&root, &object).parent().map(Path::to_path_buf));
            results.push(json!({"name":candidate.name,"version":version,"metadata":skill.metadata,"source":candidate.source}));
            state.skills.insert(
                candidate.name,
                StoredSkill {
                    source: candidate.source,
                    version,
                    object,
                    asset_base,
                },
            );
            staged.stage_candidate(skill);
        }
        let dependencies = self
            .rt
            .block_on(ai_assistant::skills::usage::dependencies(Some(&names)))
            .map_err(|error| invalid(error.to_string()))?;
        if !dependencies.is_empty() {
            return Err(RuntimeError::SkillInUse { dependencies });
        }
        let cluster = if let Some(change) = request.cluster {
            let cluster = self.prepare_cluster_with_catalog(&change.registration, &mut staged)?;
            let current = self.registries.agent_clusters.get(&cluster.id);
            let actual = current.map(super::agent_cluster_operations::cluster_version);
            if change.expected_version != actual {
                return Err(RuntimeError::ClusterVersionConflict {
                    cluster_id: cluster.id,
                    expected: change.expected_version.unwrap_or_else(|| "<absent>".into()),
                    actual: actual.unwrap_or_else(|| "<absent>".into()),
                });
            }
            self.ensure_cluster_idle(&cluster.id)?;
            state.clusters.insert(cluster.id.clone(), cluster.clone());
            Some(cluster)
        } else {
            None
        };
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("Skill store revision overflow"))?;
        let response = json!({"schema":"agent-runtime-skill-transaction/v1","committed":commit,"store_revision":if commit { state.revision } else { request.expected_store_revision },
            "skills":results,"cluster":cluster.as_ref().map(|cluster| super::resource_definitions::cluster_definition(cluster,"registered"))});
        if !commit {
            return Ok(response);
        }
        let changed_names = names.into_iter().collect::<Vec<_>>();
        plain_path(&root)?;
        fs::create_dir_all(&root).map_err(io)?;
        plain_path(&root.join("writer.lock"))?;
        let writer = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("writer.lock"))
            .map_err(io)?;
        writer
            .try_lock()
            .map_err(|error| invalid(format!("Skill store writer busy: {error}")))?;
        // Another process can publish while candidates are being validated.
        let actual = read_managed_state(&root)?.revision;
        if actual != request.expected_store_revision {
            return Err(RuntimeError::SkillStoreConflict {
                expected: request.expected_store_revision,
                actual,
            });
        }
        let mut live = self.rt.block_on(global.write());
        persist(&root, &state)?;
        #[cfg(test)]
        if FAIL_POINT.with(|point| point.replace(0)) == 2 {
            panic!("Injected process interruption after manifest commit");
        }
        // All validation and fallible IO precede this point. The durable manifest
        // contains complete cluster definitions as well as Skill source versions.
        live.publish_reloaded(&staged, &changed_names);
        if let Some(cluster) = cluster {
            self.registries
                .dynamic_agent_clusters
                .insert(cluster.id.clone());
            self.registries
                .agent_clusters
                .insert(cluster.id.clone(), cluster);
        }
        self.registries.skill_store_revision = state.revision;
        tracing::info!(store_revision = state.revision, skills = ?changed_names, "Skill transaction committed");
        Ok(response)
    }

    fn ensure_cluster_idle(&self, id: &str) -> Result<(), RuntimeError> {
        let live = self
            .rt
            .block_on(self.manager()?.list())
            .into_iter()
            .map(|info| info.conversation_id)
            .collect::<BTreeSet<_>>();
        let conversation_ids: Vec<_> = self
            .conversation_instances
            .iter()
            .filter(|(key, instance)| instance.cluster_id == id && live.contains(*key))
            .map(|(id, _)| id.clone())
            .collect();
        if !conversation_ids.is_empty() {
            return Err(RuntimeError::ClusterInUse {
                cluster_id: id.into(),
                conversation_ids,
            });
        }
        Ok(())
    }

    fn validate_candidate_references(
        &self,
        skill: &ai_assistant::Skill,
    ) -> Result<(), RuntimeError> {
        let definitions: Value = serde_json::from_str(&self.tool_definitions()?)
            .map_err(|error| invalid(error.to_string()))?;
        let tools: BTreeSet<_> = definitions["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        for tool in &skill.metadata.tools {
            if !tools.contains(tool.as_str()) {
                return Err(invalid(format!(
                    "Skill '{}' references unknown tool '{tool}'",
                    skill.metadata.name
                )));
            }
        }
        let workflows = self
            .workflow_module()?
            .list_workflow_catalog(None)
            .map_err(|error| invalid(error.to_string()))?;
        for name in &skill.metadata.workflows {
            if !workflows
                .iter()
                .any(|workflow| workflow.name == *name && workflow.production_executable)
            {
                return Err(invalid(format!(
                    "Skill references unavailable formal workflow '{name}'"
                )));
            }
        }
        Ok(())
    }
}
