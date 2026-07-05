//! 文件副本安全机制（Copy-on-Read）
//!
//! AI 永不直接写入用户源文件。首次读取时自动复制到 workspace 目录，
//!
//! ## 使用方式
//!
//! ```rust,ignore
//! // 在 define_operation 的 execute 中替换 safe_require("path")：
//! let (path, source) = match workspace::resolve_path(&args, &ctx.cache).await {
//!     Ok(v) => v,
//!     Err(e) => return Ok(e),
//! };
//! // path = 副本路径（引擎操作用这个）
//! // source = 源路径（快照 key / 用户展示用这个）
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ai_system::{AIOutput, SimpleArgs};
use crate::cache::{Cache, CacheExt};

/// cache 中存储路径映射的 key
const PATH_MAP_KEY: &str = "workspace:path_map";

/// workspace 根目录：%LOCALAPPDATA%/Lumi/workspace/
fn workspace_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("USERPROFILE").map(|u| format!("{}\\AppData\\Local", u)))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join("Lumi").join("workspace")
}

/// 计算副本文件名：{hash8}_{原文件名}
fn workspace_filename(source_path: &str) -> String {
    let hash = format!("{:x}", md5::compute(source_path.as_bytes()));
    let hash8 = &hash[..8];
    let file_name = Path::new(source_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    format!("{}_{}", hash8, file_name)
}

/// 从 cache 读取路径映射表
async fn get_path_map(cache: &Arc<dyn Cache>) -> HashMap<String, String> {
    cache
        .get::<HashMap<String, String>>(PATH_MAP_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// 将路径映射表写回 cache
async fn set_path_map(cache: &Arc<dyn Cache>, map: &HashMap<String, String>) {
    let _ = cache.set(PATH_MAP_KEY, map, None).await;
}

///
/// 返回 `(副本路径, 源路径)`。
/// - 副本路径：引擎实际操作的文件
/// - 源路径：原始路径，用于快照 key 和用户展示
pub async fn resolve_working_path(
    source_path: &str,
    cache: &Arc<dyn Cache>,
) -> Result<(String, String), String> {
    let source = source_path.to_string();

    // 1. 查映射表
    let mut map = get_path_map(cache).await;
    if let Some(existing) = map.get(&source) {
        // 副本已存在且文件还在
        if Path::new(existing).exists() {
            return Ok((existing.clone(), source));
        }
        // 副本文件被删了，重新复制
        map.remove(&source);
    }

    // 2. 源文件必须存在
    if !Path::new(&source).exists() {
        return Err(format!("文件不存在：{}", source));
    }

    // 3. 确保 workspace 目录
    let ws_dir = workspace_dir();
    if !ws_dir.exists() {
        std::fs::create_dir_all(&ws_dir).map_err(|e| format!("创建 workspace 目录失败：{}", e))?;
    }

    // 4. 复制
    let ws_path = ws_dir.join(workspace_filename(&source));
    let ws_str = ws_path.to_string_lossy().to_string();
    std::fs::copy(&source, &ws_path).map_err(|e| format!("复制文件到 workspace 失败：{}", e))?;

    // 5. 写入映射
    map.insert(source.clone(), ws_str.clone());
    set_path_map(cache, &map).await;

    Ok((ws_str, source))
}

/// 高层 helper：从 SimpleArgs 提取 `path` 参数并 resolve。
///
/// 返回 `Ok((副本路径, 源路径))` 或 `Err(AIOutput)` 可直接 `return Ok(e)`。
pub async fn resolve_path(
    args: &SimpleArgs,
    cache: &Arc<dyn Cache>,
) -> Result<(String, String), AIOutput> {
    let source = args.safe_require("path")?;
    resolve_working_path(&source, cache)
        .await
        .map_err(|e| AIOutput::error(1, format!("[失败] {}", e)))
}

/// 为创建类操作生成 workspace 路径。
///
/// 用户意图路径（如 `D:/工作/新文档.docx`）→ workspace 中的实际路径。
///
/// 返回 `(workspace 路径, 用户意图路径)`。
pub async fn create_working_path(
    intended_path: &str,
    cache: &Arc<dyn Cache>,
) -> Result<(String, String), String> {
    let intended = intended_path.to_string();

    // 确保 workspace 目录
    let ws_dir = workspace_dir();
    if !ws_dir.exists() {
        std::fs::create_dir_all(&ws_dir).map_err(|e| format!("创建 workspace 目录失败：{}", e))?;
    }

    // 生成 workspace 路径
    let ws_path = ws_dir.join(workspace_filename(&intended));
    let ws_str = ws_path.to_string_lossy().to_string();

    let mut map = get_path_map(cache).await;
    map.insert(intended.clone(), ws_str.clone());
    set_path_map(cache, &map).await;

    Ok((ws_str, intended))
}

/// 为创建类操作从 SimpleArgs 提取 `path` 并生成 workspace 路径。
///
/// 返回 `Ok((workspace 路径, 用户意图路径))` 或 `Err(AIOutput)`。
pub async fn create_path(
    args: &SimpleArgs,
    cache: &Arc<dyn Cache>,
) -> Result<(String, String), AIOutput> {
    let intended = args.safe_require("path")?;
    create_working_path(&intended, cache)
        .await
        .map_err(|e| AIOutput::error(1, format!("[失败] {}", e)))
}

///
///
/// 返回实际保存路径。
pub async fn save_as_edited(
    source_path: &str,
    cache: &Arc<dyn Cache>,
    suffix: &str,
) -> Result<String, String> {
    let map = get_path_map(cache).await;
    let ws_path = map
        .get(source_path)
        .ok_or_else(|| format!("未找到 {} 的工作副本，请先打开文件", source_path))?;

    if !Path::new(ws_path).exists() {
        return Err(format!("工作副本不存在：{}", ws_path));
    }

    // 计算输出路径
    let src = Path::new(source_path);
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    let parent = src.parent().unwrap_or(Path::new("."));

    let out_name = if ext.is_empty() {
        format!("{}{}", stem, suffix)
    } else {
        format!("{}{}.{}", stem, suffix, ext)
    };
    let out_path = parent.join(&out_name);
    let out_str = out_path.to_string_lossy().to_string();

    std::fs::copy(ws_path, &out_path).map_err(|e| format!("保存文件失败：{}", e))?;

    Ok(out_str)
}
