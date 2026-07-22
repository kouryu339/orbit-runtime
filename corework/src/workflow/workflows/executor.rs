//! 工作流模块 —— 蓝图注册表 + 本地持久化 + 即用即弃执行
//!
//! [`WorkflowsModule`] 是 L3 层 Module，通过 `unit`（[`ExecutionUnit`]）声明
//! 读写这些资源，无需持有模块引用。
//!

use corework::{
    error::{FrameworkError, Result},
    event::EventBus,
    module::{create_module, AccessMode, Module},
    workflow::blueprint_json::BlueprintJson,
    workflow::core::DataValue,
    workflow::execution::{ExecutionContext, WorkflowExecutionReport},
    workflow::{blueprint_json::BlueprintMetadata, BlueprintLoader},
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::workflow::workflows::draft::keys;

// ============================================================================
// 内部资源键（仅本模块使用）
// ============================================================================

pub(crate) const REGISTRY: &str = "wf:registry";
pub(crate) const WORKFLOWS_DIR: &str = "wf:workflows_dir";

// ============================================================================
// 工作流来源
// ============================================================================

/// 工作流来源分类
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkflowSource {
    /// 官方预置，不可变，未来从 API 拉取
    Official,
    /// 本地用户创建（录制 / AI 构建），持久化到磁盘
    Local,
}

impl Default for WorkflowSource {
    fn default() -> Self {
        Self::Local
    }
}

// ============================================================================
// 蓝图注册表项（可序列化，存入 World）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BlueprintEntry {
    pub(crate) metadata: BlueprintMetadata,
    pub(crate) file_path: String,
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) source: WorkflowSource,
    #[serde(default = "default_workflow_revision")]
    pub(crate) revision: u64,
}

fn default_workflow_revision() -> u64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisteredWorkflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub key: String,
    pub file_name: String,
    pub file_path: String,
    pub kind: crate::workflow::workflows::catalog::WorkflowResourceKind,
    pub revision: u64,
    pub trusted: bool,
    pub production_executable: bool,
}

#[derive(Debug, Clone)]
pub struct WorkflowExecutionOutcome {
    pub report: WorkflowExecutionReport,
    pub error: Option<String>,
}

// ============================================================================
// WorkflowsModule
// ============================================================================

/// 工作流模块。
///
/// - `unit`：通过 World 资源持有蓝图注册表与草稿数据的所有权
/// - `workflows_dir`：本地工作流持久化目录（`app_local_data_dir/workflows/`）
pub struct WorkflowsModule {
    pub(crate) unit: Module,
    pub(crate) workflows_dir: PathBuf,
    pub(crate) event_bus: Arc<dyn EventBus>,
}

impl WorkflowsModule {
    /// 初始化模块，声明所有 World 资源所有权并开放读写。
    ///
    /// `workflows_dir`：本地工作流 JSON 文件的持久化目录。
    pub fn new(workflows_dir: PathBuf) -> Result<Self> {
        let unit = create_module("workflows")?;
        let event_bus = unit.global_event_bus();
        Self::new_with_unit_and_event_bus(workflows_dir, unit, event_bus)
    }

    pub fn new_with_event_bus(
        workflows_dir: PathBuf,
        event_bus: Arc<dyn EventBus>,
    ) -> Result<Self> {
        let unit = create_module("workflows")?;
        Self::new_with_unit_and_event_bus(workflows_dir, unit, event_bus)
    }

    fn new_with_unit_and_event_bus(
        workflows_dir: PathBuf,
        unit: Module,
        event_bus: Arc<dyn EventBus>,
    ) -> Result<Self> {
        for key in &[
            REGISTRY,
            crate::workflow::workflows::catalog::DRAFT_REGISTRY,
            keys::DRAFT,
            keys::CURSOR,
            keys::HISTORY,
            keys::USED_NODE_DETAILS,
            WORKFLOWS_DIR,
        ] {
            unit.declare_resource_access(key, AccessMode::Owner)?;
            unit.grant_access_to(key, "*", AccessMode::ReadWrite)?;
        }

        unit.set_resource(REGISTRY, &Vec::<BlueprintEntry>::new(), None)?;
        unit.set_resource(
            crate::workflow::workflows::catalog::DRAFT_REGISTRY,
            &Vec::<crate::workflow::workflows::catalog::DraftWorkflowEntry>::new(),
            None,
        )?;
        unit.set_resource(
            WORKFLOWS_DIR,
            &workflows_dir.to_string_lossy().to_string(),
            None,
        )?;

        // 确保持久化目录存在
        std::fs::create_dir_all(&workflows_dir).map_err(|e| {
            FrameworkError::SystemError(format!(
                "创建工作流持久化目录失败 {:?}: {}",
                workflows_dir, e
            ))
        })?;

        tracing::debug!(
            "WorkflowsModule 初始化完成，持久化目录: {:?}",
            workflows_dir
        );

        Ok(Self {
            unit,
            workflows_dir,
            event_bus,
        })
    }

    /// 获取持久化目录路径
    pub fn workflows_dir(&self) -> &PathBuf {
        &self.workflows_dir
    }

    /// 创建与模块绑定的执行上下文（包含 World、Registry、EventBus）。
    pub fn create_context(&self) -> corework::orchestration::Context {
        self.unit.create_context()
    }

    // =========================================================================
    // 启动扫描：本地目录 → 自动注册
    // =========================================================================

    /// 扫描 `workflows_dir` 下工作流 JSON 文件，兼容旧的普通 `.json` 文件。
    /// 返回成功加载的数量。
    pub fn scan_local_dir(&self) -> Result<usize> {
        let entries = std::fs::read_dir(&self.workflows_dir).map_err(|e| {
            FrameworkError::SystemError(format!(
                "扫描工作流目录失败 {:?}: {}",
                self.workflows_dir, e
            ))
        })?;

        let mut count = 0;
        let mut registry = self.get_registry()?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let path_str = path.to_string_lossy().to_string();

            // 跳过已在 registry 中的（按 file_path 去重）
            if registry.iter().any(|e| e.file_path == path_str) {
                count += 1;
                continue;
            }

            match std::fs::read_to_string(&path) {
                Ok(json_str) => {
                    match BlueprintJson::from_json_str(&json_str) {
                        Ok(blueprint) => {
                            if let Err(e) = blueprint.validate() {
                                tracing::warn!("跳过无效的工作流文件 {:?}: {}", path, e);
                                continue;
                            }
                            let key = Self::make_key(&blueprint.metadata);
                            // 按 key 去重（可能同名工作流被改过文件名）
                            registry.retain(|e| e.key != key);
                            registry.push(BlueprintEntry {
                                metadata: blueprint.metadata,
                                file_path: path_str,
                                key,
                                source: WorkflowSource::Local,
                                revision: 1,
                            });
                            count += 1;
                        }
                        Err(e) => {
                            tracing::warn!("跳过无法解析的工作流文件 {:?}: {}", path, e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("跳过无法读取的工作流文件 {:?}: {}", path, e);
                }
            }
        }

        self.unit.set_resource(REGISTRY, &registry, None)?;
        tracing::debug!(count, "local workflows scanned");
        Ok(count)
    }

    // =========================================================================
    // =========================================================================

    /// 保存 BlueprintJson 到本地持久化目录并注册到 registry。
    /// 返回注册 key。
    ///
    pub fn save_local(&self, blueprint: &BlueprintJson) -> Result<String> {
        let mut blueprint = blueprint.clone();
        if blueprint.metadata.id.is_empty() {
            blueprint.metadata.id = blueprint.metadata.name.clone();
        }
        blueprint.normalize_node_sizes();
        let safe_name = blueprint
            .metadata
            .name
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let file_name = format!("{}.workflow.json", safe_name);
        let save_path = self.workflows_dir.join(&file_name);

        let json_pretty = serde_json::to_string_pretty(&blueprint).map_err(|e| {
            FrameworkError::SystemError(format!("序列化 BlueprintJson 失败: {}", e))
        })?;

        std::fs::write(&save_path, &json_pretty).map_err(|e| {
            FrameworkError::SystemError(format!("保存工作流文件失败 {:?}: {}", save_path, e))
        })?;

        let file_path_str = save_path.to_string_lossy().to_string();
        let key = Self::make_key(&blueprint.metadata);

        // 注册到 registry
        let mut registry = self.get_registry()?;
        registry.retain(|e| e.key != key && e.metadata.name != blueprint.metadata.name);
        registry.push(BlueprintEntry {
            metadata: blueprint.metadata.clone(),
            file_path: file_path_str.clone(),
            key: key.clone(),
            source: WorkflowSource::Local,
            revision: 1,
        });
        self.unit.set_resource(REGISTRY, &registry, None)?;

        tracing::debug!(
            "save_local: 工作流「{}」已保存并注册 → {}",
            blueprint.metadata.name,
            file_path_str
        );
        Ok(key)
    }

    // =========================================================================
    // 蓝图注册表查询
    // =========================================================================

    /// 从文件加载蓝图并注册（用于手动加载外部文件）。
    pub async fn load_from_file(&self, file_path: impl AsRef<std::path::Path>) -> Result<String> {
        let path = file_path.as_ref();
        let path_str = path.to_string_lossy().to_string();

        // 验证可加载
        let _ = BlueprintLoader::new().load_workflow_from_file(path).await?;

        let json_str = std::fs::read_to_string(path)
            .map_err(|e| FrameworkError::SystemError(format!("读取文件失败: {e}")))?;
        let blueprint = BlueprintJson::from_json_str(&json_str)
            .map_err(|e| FrameworkError::SystemError(format!("JSON解析失败: {e}")))?;

        let key = Self::make_key(&blueprint.metadata);
        let mut registry = self.get_registry()?;
        registry.retain(|e| e.key != key);
        registry.push(BlueprintEntry {
            metadata: blueprint.metadata,
            file_path: path_str,
            key: key.clone(),
            source: WorkflowSource::Local,
            revision: 1,
        });
        self.unit.set_resource(REGISTRY, &registry, None)?;

        tracing::debug!(workflow_key = %key, "workflow loaded");
        Ok(key)
    }

    /// 列出所有已登记的蓝图元数据。
    pub fn list(&self) -> Result<Vec<BlueprintMetadata>> {
        Ok(self
            .get_registry()?
            .into_iter()
            .map(|e| e.metadata)
            .collect())
    }

    /// 列出所有已登记蓝图的 key。
    pub fn list_keys(&self) -> Result<Vec<String>> {
        Ok(self.get_registry()?.into_iter().map(|e| e.key).collect())
    }

    pub fn registered_workflows(&self) -> Result<Vec<RegisteredWorkflow>> {
        let mut workflows = self
            .get_registry()?
            .into_iter()
            .map(Self::registered_workflow)
            .collect::<Vec<_>>();
        workflows.sort_by(|left, right| left.id.cmp(&right.id).then(left.name.cmp(&right.name)));
        Ok(workflows)
    }

    pub fn register_resource(&self, blueprint: &BlueprintJson) -> Result<RegisteredWorkflow> {
        self.persist_resource(blueprint, false, None)
    }

    pub fn update_resource(&self, blueprint: &BlueprintJson) -> Result<RegisteredWorkflow> {
        self.persist_resource(blueprint, true, None)
    }

    pub fn delete_resource(&self, id: &str) -> Result<RegisteredWorkflow> {
        let id = Self::validate_resource_id(id)?;
        let registry = self.get_registry()?;
        let entry = registry
            .iter()
            .find(|entry| entry.metadata.id == id)
            .cloned()
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "workflow resource '{}' does not exist",
                    id
                ))
            })?;
        let path = PathBuf::from(&entry.file_path);
        self.ensure_owned_resource_path(&path)?;

        let next_registry = registry
            .iter()
            .filter(|candidate| candidate.metadata.id != id)
            .cloned()
            .collect::<Vec<_>>();
        self.unit.set_resource(REGISTRY, &next_registry, None)?;
        if let Err(error) = std::fs::remove_file(&path) {
            let _ = self.unit.set_resource(REGISTRY, &registry, None);
            return Err(FrameworkError::SystemError(format!(
                "delete workflow resource file '{}' failed: {}",
                path.display(),
                error
            )));
        }

        Ok(Self::registered_workflow(entry))
    }

    /// 获取指定蓝图的元数据。
    pub fn metadata(&self, key: &str) -> Result<Option<BlueprintMetadata>> {
        Ok(self
            .get_registry()?
            .into_iter()
            .find(|e| e.key == key)
            .map(|e| e.metadata))
    }

    /// 卸载蓝图（从注册表移除；Local 工作流同时删除文件）。
    pub fn unload(&self, key: &str) -> Result<()> {
        let mut reg = self.get_registry()?;
        let entry = reg.iter().find(|e| e.key == key).cloned();
        let before = reg.len();
        reg.retain(|e| e.key != key);
        if reg.len() == before {
            return Err(FrameworkError::InvalidOperation(format!(
                "蓝图 {key} 不存在"
            )));
        }
        self.unit.set_resource(REGISTRY, &reg, None)?;

        // Local 工作流同时删除文件
        if let Some(e) = entry {
            if e.source == WorkflowSource::Local && !e.file_path.is_empty() {
                let _ = std::fs::remove_file(&e.file_path);
            }
        }

        tracing::debug!(workflow_key = %key, "workflow unloaded");
        Ok(())
    }

    /// 重新从原始文件加载蓝图。
    pub async fn reload(&self, key: &str) -> Result<()> {
        let path = self
            .get_registry()?
            .into_iter()
            .find(|e| e.key == key)
            .ok_or_else(|| FrameworkError::InvalidOperation(format!("蓝图 {key} 不存在")))?
            .file_path;
        if path.is_empty() {
            return Err(FrameworkError::InvalidOperation(format!(
                "蓝图 {key} 无关联文件，无法重新加载"
            )));
        }
        self.unload(key)?;
        self.load_from_file(&path).await?;
        Ok(())
    }

    // =========================================================================
    // 蓝图执行（即用即弃）
    // =========================================================================

    /// 按名称执行工作流：从文件加载 → 创建实例 → 执行 → 丢弃。
    pub async fn execute_by_name(
        &self,
        name: &str,
        inputs: HashMap<String, JsonValue>,
    ) -> Result<HashMap<String, JsonValue>> {
        let entry = self
            .get_registry()?
            .into_iter()
            .find(|e| e.metadata.name == name)
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!("未找到名为「{}」的工作流", name))
            })?;

        if entry.file_path.is_empty() {
            return Err(FrameworkError::InvalidOperation(format!(
                "工作流「{}」无关联文件，无法执行",
                name
            )));
        }

        self.execute_from_file(&entry.file_path, inputs).await
    }

    pub async fn execute_registered_report(
        &self,
        selector: &str,
        inputs: HashMap<String, JsonValue>,
        trace_enabled: bool,
    ) -> Result<WorkflowExecutionReport> {
        let selector = selector.trim();
        if selector.is_empty() {
            return Err(FrameworkError::InvalidOperation(
                "workflow selector must not be empty".to_string(),
            ));
        }
        let entry = self
            .get_registry()?
            .into_iter()
            .find(|entry| {
                entry.metadata.id == selector
                    || entry.metadata.name == selector
                    || entry.key == selector
            })
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "registered workflow '{}' does not exist",
                    selector
                ))
            })?;
        let blueprint = BlueprintJson::from_workflow_file(&entry.file_path)
            .map_err(FrameworkError::SystemError)?;
        self.execute_from_blueprint_report(blueprint, inputs, trace_enabled)
            .await
    }

    pub async fn execute_registered_outcome(
        &self,
        selector: &str,
        inputs: HashMap<String, JsonValue>,
    ) -> Result<WorkflowExecutionOutcome> {
        let selector = selector.trim();
        if selector.is_empty() {
            return Err(FrameworkError::InvalidOperation(
                "workflow selector must not be empty".to_string(),
            ));
        }
        let entry = self
            .get_registry()?
            .into_iter()
            .find(|entry| {
                entry.metadata.id == selector
                    || entry.metadata.name == selector
                    || entry.key == selector
            })
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "registered workflow '{}' does not exist",
                    selector
                ))
            })?;
        let blueprint = BlueprintJson::from_workflow_file(&entry.file_path)
            .map_err(FrameworkError::SystemError)?;
        self.execute_from_blueprint_outcome(blueprint, inputs).await
    }

    /// 从文件加载并执行工作流（即用即弃）。
    pub async fn execute_from_file(
        &self,
        file_path: &str,
        inputs: HashMap<String, JsonValue>,
    ) -> Result<HashMap<String, JsonValue>> {
        let workflow_inputs = inputs
            .into_iter()
            .map(|(k, v)| (k, corework::workflow::core::DataValue::new("JsonValue", v)))
            .collect();

        tracing::debug!(file_path = %file_path, "workflow execution started");
        let t = std::time::Instant::now();
        let mut wf = BlueprintLoader::new()
            .load_workflow_from_file(file_path)
            .await?;
        let outputs = wf.execute(workflow_inputs).await?;
        tracing::debug!(
            file_path = %file_path,
            duration_ms = t.elapsed().as_millis(),
            "workflow execution completed"
        );

        Ok(outputs
            .into_iter()
            .map(|(k, v)| (k, v.json_value().clone()))
            .collect())
    }

    /// 从 BlueprintJson 直接执行临时工作流。
    pub async fn execute_from_blueprint(
        &self,
        blueprint: BlueprintJson,
        inputs: HashMap<String, JsonValue>,
    ) -> Result<HashMap<String, JsonValue>> {
        let workflow_inputs = inputs
            .into_iter()
            .map(|(k, v)| (k, DataValue::new("JsonValue", v)))
            .collect();

        tracing::debug!(
            workflow_name = %blueprint.metadata.name,
            "inline workflow execution started"
        );
        let t = std::time::Instant::now();
        let ctx = self.create_context();
        let loaded = BlueprintLoader::new().load_from_blueprint_json(blueprint, &ctx)?;
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        loaded.compiled.initialize_defaults(&mut exec_ctx).await?;
        let executor = loaded.compiled.executor();
        let outputs = executor
            .execute_with_params(&mut exec_ctx, workflow_inputs)
            .await?;
        tracing::debug!(
            duration_ms = t.elapsed().as_millis(),
            "inline workflow execution completed"
        );

        Ok(outputs
            .into_iter()
            .map(|(k, v)| (k, v.json_value().clone()))
            .collect())
    }

    /// 从 BlueprintJson 直接执行完整路径，并返回 outputs + 可选结构化 trace。
    pub async fn execute_from_blueprint_report(
        &self,
        blueprint: BlueprintJson,
        inputs: HashMap<String, JsonValue>,
        trace_enabled: bool,
    ) -> Result<WorkflowExecutionReport> {
        let workflow_inputs = inputs
            .into_iter()
            .map(|(k, v)| (k, DataValue::new("JsonValue", v)))
            .collect();

        let workflow_name = blueprint.metadata.name.clone();
        tracing::debug!(workflow_name = %workflow_name, "inline workflow report started");
        let ctx = self.create_context();
        let loaded = BlueprintLoader::new().load_from_blueprint_json(blueprint, &ctx)?;
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        if trace_enabled {
            exec_ctx.enable_trace(workflow_name, loaded.compiled.source_map.clone());
        }
        loaded.compiled.initialize_defaults(&mut exec_ctx).await?;
        let executor = loaded.compiled.executor();
        let outputs = executor
            .execute_with_params(&mut exec_ctx, workflow_inputs)
            .await?;
        let trace = exec_ctx.take_trace();

        Ok(WorkflowExecutionReport { outputs, trace })
    }

    pub async fn execute_from_blueprint_outcome(
        &self,
        blueprint: BlueprintJson,
        inputs: HashMap<String, JsonValue>,
    ) -> Result<WorkflowExecutionOutcome> {
        let workflow_inputs = inputs
            .into_iter()
            .map(|(key, value)| (key, DataValue::new("JsonValue", value)))
            .collect();
        let workflow_name = blueprint.metadata.name.clone();
        let ctx = self.create_context();
        let loaded = BlueprintLoader::new().load_from_blueprint_json(blueprint, &ctx)?;
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        exec_ctx.enable_trace(workflow_name, loaded.compiled.source_map.clone());

        if let Err(error) = loaded.compiled.initialize_defaults(&mut exec_ctx).await {
            return Ok(WorkflowExecutionOutcome {
                report: WorkflowExecutionReport {
                    outputs: HashMap::new(),
                    trace: exec_ctx.take_trace(),
                },
                error: Some(error.to_string()),
            });
        }

        let execution = loaded
            .compiled
            .executor()
            .execute_with_params(&mut exec_ctx, workflow_inputs)
            .await;
        let trace = exec_ctx.take_trace();
        match execution {
            Ok(outputs) => Ok(WorkflowExecutionOutcome {
                report: WorkflowExecutionReport { outputs, trace },
                error: None,
            }),
            Err(error) => Ok(WorkflowExecutionOutcome {
                report: WorkflowExecutionReport {
                    outputs: HashMap::new(),
                    trace,
                },
                error: Some(error.to_string()),
            }),
        }
    }

    /// 验证输入是否满足蓝图要求，返回错误列表（空列表表示合法）。
    pub fn validate_inputs(
        &self,
        key: &str,
        inputs: &HashMap<String, JsonValue>,
    ) -> Result<Vec<String>> {
        let meta = self
            .metadata(key)?
            .ok_or_else(|| FrameworkError::InvalidOperation(format!("蓝图 {key} 不存在")))?;
        Ok(meta
            .inputs
            .iter()
            .filter(|p| !inputs.contains_key(&p.name) && p.default_value.is_none())
            .map(|p| format!("缺少必填参数: {}", p.name))
            .collect())
    }

    // =========================================================================
    // 草稿辅助入口（供 Tauri 层直接调用）
    // =========================================================================

    /// 初始化新草稿，清空旧草稿与历史。
    pub fn draft_new(&self, name: &str) -> Result<()> {
        self.unit.set_resource(
            keys::DRAFT,
            &crate::workflow::workflows::draft::WorkflowDraft::new(name),
            None,
        )?;
        self.unit
            .set_resource(keys::CURSOR, &Option::<String>::None, None)?;
        self.unit.set_resource(
            keys::HISTORY,
            &Vec::<crate::workflow::workflows::draft::WorkflowDraft>::new(),
            None,
        )?;
        self.unit.set_resource(
            keys::USED_NODE_DETAILS,
            &HashMap::<String, String>::new(),
            None,
        )?;
        // P0 同步：新建草稿也要让 snapshot 立即可见（首版本 v1）
        let ctx = self.create_context();
        let _ = crate::workflow::workflows::snapshot::refresh_world_snapshot(&ctx);
        Ok(())
    }

    /// 获取当前草稿快照（只读）。
    pub fn draft_get(&self) -> Result<Option<crate::workflow::workflows::draft::WorkflowDraft>> {
        self.unit.get_resource(keys::DRAFT)
    }

    /// 将修改后的草稿写回。
    pub fn draft_put(
        &self,
        draft: &crate::workflow::workflows::draft::WorkflowDraft,
    ) -> Result<()> {
        self.unit.set_resource(keys::DRAFT, draft, None)?;
        // P0 同步：底层写回是加载/保存等场景的入口，必须刷新快照
        let ctx = self.create_context();
        let _ = crate::workflow::workflows::snapshot::refresh_world_snapshot(&ctx);
        Ok(())
    }

    /// 便捷方法：获取当前草稿的 BlueprintJson。
    pub fn draft_get_blueprint(&self) -> Result<Option<BlueprintJson>> {
        Ok(self.draft_get()?.map(|d| d.blueprint))
    }

    /// 便捷方法：获取当前草稿的操作链文本。
    pub fn draft_get_chain_text(&self) -> Result<Option<String>> {
        Ok(self.draft_get()?.map(|d| d.chain_text))
    }

    pub async fn draft_commit(&self) -> Result<String> {
        let draft: crate::workflow::workflows::draft::WorkflowDraft = self
            .unit
            .get_resource(keys::DRAFT)?
            .ok_or_else(|| FrameworkError::InvalidOperation("当前无草稿".into()))?;

        // 验证可执行性
        let json = serde_json::to_string(&draft.blueprint)
            .map_err(|e| FrameworkError::SystemError(format!("序列化失败: {e}")))?;
        let _ = BlueprintLoader::new()
            .load_workflow_from_json_str(&json)
            .await?;

        // 保存到本地目录并注册
        let key = self.save_local(&draft.blueprint)?;
        tracing::debug!(workflow_key = %key, "workflow draft committed");
        Ok(key)
    }

    // =========================================================================
    // 私有辅助
    // =========================================================================

    pub(crate) fn get_registry(&self) -> Result<Vec<BlueprintEntry>> {
        Ok(self.unit.get_resource(REGISTRY)?.unwrap_or_default())
    }

    pub(crate) fn persist_resource(
        &self,
        blueprint: &BlueprintJson,
        update: bool,
        revision: Option<u64>,
    ) -> Result<RegisteredWorkflow> {
        let mut blueprint = blueprint.clone();
        let id = Self::validate_resource_id(&blueprint.metadata.id)?;
        let name = blueprint.metadata.name.trim().to_string();
        if name.is_empty() {
            return Err(FrameworkError::InvalidOperation(
                "workflow resource name must not be empty".to_string(),
            ));
        }
        blueprint.metadata.id = id.clone();
        blueprint.metadata.name = name.clone();
        blueprint.normalize_node_sizes();
        blueprint
            .validate()
            .map_err(FrameworkError::InvalidOperation)?;

        let registry = self.get_registry()?;
        let existing = registry
            .iter()
            .find(|entry| entry.metadata.id == id)
            .cloned();
        if update && existing.is_none() {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow resource '{}' does not exist",
                id
            )));
        }
        if !update && existing.is_some() {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow resource '{}' already exists",
                id
            )));
        }
        if registry
            .iter()
            .any(|entry| entry.metadata.id != id && entry.metadata.name == name)
        {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow resource name '{}' already exists",
                name
            )));
        }
        self.ensure_name_available(&name, Some(&id))?;

        let path = match existing.as_ref() {
            Some(entry) => PathBuf::from(&entry.file_path),
            None => self.workflows_dir.join(format!("{}.workflow.json", id)),
        };
        self.ensure_owned_resource_path(&path)?;
        if !update && path.exists() {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow resource file '{}' already exists",
                path.display()
            )));
        }

        let json = serde_json::to_string_pretty(&blueprint).map_err(|error| {
            FrameworkError::SystemError(format!("serialize workflow resource failed: {error}"))
        })?;
        let previous_file = std::fs::read(&path).ok();
        std::fs::write(&path, json).map_err(|error| {
            FrameworkError::SystemError(format!(
                "write workflow resource file '{}' failed: {}",
                path.display(),
                error
            ))
        })?;

        let entry = BlueprintEntry {
            metadata: blueprint.metadata.clone(),
            file_path: path.to_string_lossy().to_string(),
            key: Self::make_key(&blueprint.metadata),
            source: WorkflowSource::Local,
            revision: revision.unwrap_or_else(|| {
                existing
                    .as_ref()
                    .map(|entry| entry.revision.saturating_add(1))
                    .unwrap_or(1)
            }),
        };
        let mut next_registry = registry;
        next_registry
            .retain(|candidate| candidate.metadata.id != id && candidate.metadata.name != name);
        next_registry.push(entry.clone());
        if let Err(error) = self.unit.set_resource(REGISTRY, &next_registry, None) {
            match previous_file {
                Some(content) => {
                    let _ = std::fs::write(&path, content);
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
            return Err(error);
        }
        Ok(Self::registered_workflow(entry))
    }

    pub(crate) fn registered_workflow(entry: BlueprintEntry) -> RegisteredWorkflow {
        let file_name = PathBuf::from(&entry.file_path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        RegisteredWorkflow {
            id: entry.metadata.id,
            name: entry.metadata.name,
            description: entry.metadata.description,
            key: entry.key,
            file_name,
            file_path: entry.file_path,
            kind: crate::workflow::workflows::catalog::WorkflowResourceKind::Registered,
            revision: entry.revision,
            trusted: true,
            production_executable: true,
        }
    }

    pub(crate) fn validate_resource_id(id: &str) -> Result<String> {
        let id = id.trim();
        if id.is_empty()
            || !id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        {
            return Err(FrameworkError::InvalidOperation(
                "workflow resource id must contain only ASCII letters, digits, '.', '_' or '-'"
                    .to_string(),
            ));
        }
        Ok(id.to_string())
    }

    fn ensure_owned_resource_path(&self, path: &std::path::Path) -> Result<()> {
        if path.parent() != Some(self.workflows_dir.as_path()) {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow resource file '{}' is outside the configured workflow directory",
                path.display()
            )));
        }
        Ok(())
    }

    fn make_key(meta: &BlueprintMetadata) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        meta.name.hash(&mut h);
        meta.created.hash(&mut h);
        format!("{}:{:x}", meta.name, h.finish() as u32)
    }
}
