//! 草稿快照渲染器
//!
//! 把 `WorkflowDraft` 渲染成 AI 友好的文本快照，供 recorder agent 的 system prompt 使用。
//!
//! # 输出格式
//!
//! ```text
//! === CONTRACT ===
//! inputs:
//!   user_id : string              # 用户ID
//!   task    : string              #
//!
//! returns:
//!   result  : json                # 最终产物
//!
//! vars:
//!   counter   : int  = 0          # 循环计数器
//!
//! === SCRIPT ===
//! 1  open_browser url=$(input.task)
//! 2  click_element selector="#submit"
//! ```
//!
//! # 设计原则
//!
//! - **契约三段分明**：inputs / returns / vars 各占一块，空块渲染 `(empty)` 而非省略
//! - **注释是一等公民**：`description` 字段映射为 `# ...`，空注释渲染 `#` 占位
//! - **对齐排版**：三列（name / type / default）左对齐，AI 视觉扫描更快
//! - **命名映射**：内部 `outputs` 在快照中对外名为 `returns`
//!
//! # 版本号
//!
//! `DraftSnapshot.version` 单调递增，用于：
//! 1. 乐观锁：`WriteScript(base_version=N)` 防止覆盖他人改动
//! 2. 事件去重：前端比较版本号跳过过期刷新

use corework::workflow::blueprint_json::{BlueprintJson, BlueprintVariable, PinMetadata};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::workflow::workflows::draft::{keys, WorkflowDraft};

// ============================================================================
// 对外类型
// ============================================================================

/// 草稿快照 —— AI 上下文中的"当前真相"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSnapshot {
    /// 渲染后的文本（可直接塞入 prompt）
    pub text: String,
    /// 单调递增版本号
    pub version: u64,
}

// ============================================================================
// World cache 集成
// ============================================================================

/// 从 World 读取当前快照版本号。
///
/// 草稿首次创建前返回 0；每次 `refresh_world_snapshot` 成功后自增。
pub fn current_version(ctx: &corework::orchestration::Context) -> u64 {
    let Ok(world) = ctx.get_world_cache() else {
        return 0;
    };
    world
        .get_resource::<u64>(keys::SNAPSHOT_VERSION)
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// 从 World 读取当前快照（如果存在）。
pub fn current_snapshot(ctx: &corework::orchestration::Context) -> Option<DraftSnapshot> {
    let world = ctx.get_world_cache().ok()?;
    world
        .get_resource::<DraftSnapshot>(keys::SNAPSHOT)
        .ok()
        .flatten()
}

/// **草稿变更后必须调用**。
///
/// 1. 从 World 读当前草稿
/// 2. 版本号 +1
/// 3. 渲染新快照并写回 World
/// 4. 通过全局 EventBus 广播 `ui:snapshot-updated`（Tauri 层会转发给前端）
/// 5. 返回新快照供调用方使用
///
/// 若 World 无草稿则直接返回 `None`，不报错（保持调用点容错）。
pub fn refresh_world_snapshot(
    ctx: &corework::orchestration::Context,
) -> Result<Option<DraftSnapshot>, corework::prelude::FrameworkError> {
    let world = ctx.get_world_cache()?;

    let Some(draft) = world.get_resource::<WorkflowDraft>(keys::DRAFT)? else {
        return Ok(None);
    };

    let prev: u64 = world
        .get_resource::<u64>(keys::SNAPSHOT_VERSION)?
        .unwrap_or(0);
    let next = prev.wrapping_add(1);

    let snap = render_snapshot(&draft, next);
    world.set_resource(keys::SNAPSHOT_VERSION, &next, None)?;
    world.set_resource(keys::SNAPSHOT, &snap, None)?;

    broadcast_snapshot_updated(&snap);

    Ok(Some(snap))
}

/// 异步广播 `ui:snapshot-updated` 到全局 EventBus。
///
/// 采用 fire-and-forget：失败只记日志，不影响草稿写入。
fn broadcast_snapshot_updated(snap: &DraftSnapshot) {
    let payload = serde_json::json!({
        "snapshot_text": snap.text,
        "version": snap.version,
    });
    let text = snap.text.clone();
    let version = snap.version;

    // FrameworkState 全局初始化；若尚未初始化则静默返回
    let Ok(framework) = corework::world::FrameworkState::initialize() else {
        return;
    };
    let event_bus = framework.event_bus();

    tokio::spawn(async move {
        use corework::event::EventBus as _;
        let event = corework::event::BaseEvent::new(
            "ui:snapshot-updated",
            serde_json::json!({
                "snapshot_text": text,
                "version": version,
            }),
        );
        let _ = event_bus.publish(event).await;
        drop(payload);
    });
}

// ============================================================================
// 渲染入口
// ============================================================================

/// 把 `WorkflowDraft` 渲染为文本快照。
///
/// `version` 由调用方传入（通常来自 `WorkflowDraft::mark_mutated` 的返回值）。
pub fn render_snapshot(draft: &WorkflowDraft, version: u64) -> DraftSnapshot {
    let mut out = String::new();

    render_contract(&mut out, &draft.blueprint);
    out.push('\n');
    render_script(&mut out, &draft.chain_text);

    DraftSnapshot { text: out, version }
}

// ============================================================================
// 契约块渲染
// ============================================================================

fn render_contract(out: &mut String, bp: &BlueprintJson) {
    out.push_str("=== CONTRACT ===\n");

    // inputs
    out.push_str("inputs:\n");
    if bp.metadata.inputs.is_empty() {
        out.push_str("  (empty)\n");
    } else {
        render_pins(out, &bp.metadata.inputs);
    }
    out.push('\n');

    // returns（内部字段是 outputs）
    out.push_str("returns:\n");
    if bp.metadata.outputs.is_empty() {
        out.push_str("  (empty)\n");
    } else {
        render_pins(out, &bp.metadata.outputs);
    }
    out.push('\n');

    // vars
    out.push_str("vars:\n");
    if bp.variables.is_empty() {
        out.push_str("  (empty)\n");
    } else {
        render_vars(out, &bp.variables);
    }
}

fn render_pins(out: &mut String, pins: &[PinMetadata]) {
    // 两列对齐：name / type
    let name_w = pins.iter().map(|p| p.name.len()).max().unwrap_or(0);
    let type_w = pins.iter().map(|p| p.data_type.len()).max().unwrap_or(0);

    for p in pins {
        // 默认值（如果有）
        let default_part = match &p.default_value {
            Some(v) => format!(" = {}", fmt_default(v)),
            None => String::new(),
        };
        let comment = fmt_comment(&p.description);
        out.push_str(&format!(
            "  {:<nw$} : {:<tw$}{}  {}\n",
            p.name,
            p.data_type,
            default_part,
            comment,
            nw = name_w,
            tw = type_w,
        ));
    }
}

fn render_vars(out: &mut String, vars: &[BlueprintVariable]) {
    let name_w = vars.iter().map(|v| v.name.len()).max().unwrap_or(0);
    let type_w = vars.iter().map(|v| v.data_type.len()).max().unwrap_or(0);

    for v in vars {
        let default_part = match &v.default_value {
            Some(val) => format!(" = {}", fmt_default(val)),
            None => String::new(),
        };
        let comment = fmt_comment(&v.description);
        out.push_str(&format!(
            "  {:<nw$} : {:<tw$}{}  {}\n",
            v.name,
            v.data_type,
            default_part,
            comment,
            nw = name_w,
            tw = type_w,
        ));
    }
}

fn fmt_default(v: &JsonValue) -> String {
    match v {
        JsonValue::String(s) => format!("\"{}\"", s),
        other => other.to_string(),
    }
}

/// 空注释渲染为 `#`（提示 AI 可以补），非空渲染为 `# text`
fn fmt_comment(desc: &str) -> String {
    if desc.is_empty() {
        "#".to_string()
    } else {
        format!("# {}", desc)
    }
}

// ============================================================================
// 脚本块渲染
// ============================================================================

fn render_script(out: &mut String, chain_text: &str) {
    out.push_str("=== SCRIPT ===\n");
    if chain_text.trim().is_empty() {
        out.push_str("(empty)\n");
    } else {
        out.push_str(chain_text);
        if !chain_text.ends_with('\n') {
            out.push('\n');
        }
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use corework::workflow::blueprint_json::{BlueprintJson, BlueprintVariable, PinMetadata};
    use serde_json::json;

    fn empty_draft() -> WorkflowDraft {
        WorkflowDraft {
            blueprint: BlueprintJson::new("test"),
            chain_text: String::new(),
        }
    }

    #[test]
    fn empty_draft_renders_three_empty_blocks_and_empty_script() {
        let snap = render_snapshot(&empty_draft(), 1);
        assert!(snap.text.contains("inputs:\n  (empty)"));
        assert!(snap.text.contains("returns:\n  (empty)"));
        assert!(snap.text.contains("vars:\n  (empty)"));
        assert!(snap.text.contains("=== SCRIPT ===\n(empty)"));
        assert_eq!(snap.version, 1);
    }

    #[test]
    fn outputs_field_is_rendered_as_returns() {
        let mut d = empty_draft();
        d.blueprint.metadata.outputs.push(PinMetadata {
            name: "result".into(),
            data_type: "json".into(),
            description: "最终产物".into(),
            default_value: None,
        });
        let snap = render_snapshot(&d, 2);
        assert!(snap.text.contains("returns:"));
        assert!(!snap.text.contains("outputs:"));
        assert!(snap.text.contains("result : json"));
        assert!(snap.text.contains("# 最终产物"));
    }

    #[test]
    fn empty_description_renders_hash_placeholder() {
        let mut d = empty_draft();
        d.blueprint.metadata.inputs.push(PinMetadata {
            name: "x".into(),
            data_type: "string".into(),
            description: String::new(),
            default_value: None,
        });
        let snap = render_snapshot(&d, 1);
        // 应该有 "#" 占位（不带文本）
        let line = snap
            .text
            .lines()
            .find(|l| l.trim_start().starts_with("x "))
            .expect("input line exists");
        assert!(line.trim_end().ends_with('#'), "line was: {:?}", line);
    }

    #[test]
    fn var_with_default_renders_equals() {
        let mut d = empty_draft();
        d.blueprint.variables.push(BlueprintVariable {
            name: "counter".into(),
            data_type: "int".into(),
            default_value: Some(json!(0)),
            description: "循环计数".into(),
        });
        let snap = render_snapshot(&d, 1);
        assert!(snap.text.contains("counter : int = 0"));
        assert!(snap.text.contains("# 循环计数"));
    }

    #[test]
    fn string_default_is_quoted() {
        let mut d = empty_draft();
        d.blueprint.variables.push(BlueprintVariable {
            name: "msg".into(),
            data_type: "string".into(),
            default_value: Some(json!("hello")),
            description: String::new(),
        });
        let snap = render_snapshot(&d, 1);
        assert!(snap.text.contains("= \"hello\""), "got: {}", snap.text);
    }

    #[test]
    fn script_block_preserves_chain_text() {
        let mut d = empty_draft();
        d.chain_text = "1  open_browser url=$(input.task)\n2  click_element".into();
        let snap = render_snapshot(&d, 1);
        assert!(snap.text.contains("=== SCRIPT ===\n1  open_browser"));
        assert!(snap.text.contains("2  click_element"));
        // 确保末尾有换行
        assert!(snap.text.ends_with('\n'));
    }

    #[test]
    fn version_is_preserved() {
        let snap = render_snapshot(&empty_draft(), 42);
        assert_eq!(snap.version, 42);
    }

    #[test]
    fn alignment_with_multiple_inputs() {
        let mut d = empty_draft();
        d.blueprint.metadata.inputs.push(PinMetadata {
            name: "a".into(),
            data_type: "string".into(),
            description: String::new(),
            default_value: None,
        });
        d.blueprint.metadata.inputs.push(PinMetadata {
            name: "long_name".into(),
            data_type: "int".into(),
            description: String::new(),
            default_value: None,
        });
        let snap = render_snapshot(&d, 1);
        // 两行应该对齐：冒号位置一致
        let lines: Vec<&str> = snap.text.lines().filter(|l| l.contains(" : ")).collect();
        assert_eq!(lines.len(), 2);
        let colon_pos_0 = lines[0].find(" : ").unwrap();
        let colon_pos_1 = lines[1].find(" : ").unwrap();
        assert_eq!(colon_pos_0, colon_pos_1, "columns not aligned");
    }

    #[test]
    fn full_example_snapshot() {
        let mut d = empty_draft();
        d.blueprint.metadata.inputs.push(PinMetadata {
            name: "user_id".into(),
            data_type: "string".into(),
            description: "用户ID".into(),
            default_value: None,
        });
        d.blueprint.metadata.inputs.push(PinMetadata {
            name: "task".into(),
            data_type: "string".into(),
            description: String::new(),
            default_value: None,
        });
        d.blueprint.metadata.outputs.push(PinMetadata {
            name: "result".into(),
            data_type: "json".into(),
            description: "最终产物".into(),
            default_value: None,
        });
        d.blueprint.variables.push(BlueprintVariable {
            name: "counter".into(),
            data_type: "int".into(),
            default_value: Some(json!(0)),
            description: "循环计数器".into(),
        });
        d.chain_text = "1  open_browser url=$(input.task)".into();

        let snap = render_snapshot(&d, 7);
        // 可在 CI 中快照冻结；此处只做结构断言
        assert!(snap.text.starts_with("=== CONTRACT ==="));
        assert!(snap.text.contains("inputs:\n  user_id"));
        assert!(snap.text.contains("task    : string"));
        assert!(snap.text.contains("returns:\n  result : json"));
        assert!(snap.text.contains("vars:\n  counter : int = 0"));
        assert!(snap.text.contains("=== SCRIPT ===\n1  open_browser"));
    }
}
