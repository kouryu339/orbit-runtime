# 宿主授权目录 Grep（仅 AI）

在 Runtime 启动前，使用现有资源注册接口载入 `resources.json`：

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "id": "local-resources",
  "skills": { "root_dir": "./skills", "builtin_system": true },
  "grep": {
    "roots": [{ "id": "project", "path": "E:/open-source/ai-framework" }],
    "timeout_ms": 10000,
    "max_results": 200,
    "max_output_bytes": 65536,
    "max_file_bytes": 2097152,
    "max_pattern_bytes": 16384,
    "max_regex_bytes": 8388608,
    "max_line_bytes": 4096,
    "max_entries": 100000,
    "max_concurrent_searches": 2
  }
}
```

以上限制均为可配置默认值，必须为正；输出预算至少为 1024 字节。
根目录必须已存在，ID 为 1–64 个 ASCII 字母、数字、下划线或连字符，不得重复。
从文件注册时，相对根路径基于资源文件目录解析；直接注册 JSON 时使用绝对路径。
未配置 roots 就没有本地文件权限。授权由宿主提供，不从 AI 可写缓存或恢复快照加载。
资源注册在 Runtime 启动后冻结；变更授权须使用新配置重启 Runtime 并重建会话。

默认 thinking / thinking-pro 技能已提供 `Grep`。自定义 Skill 将其加入 tools 即可，
仍受普通工具权限规则约束。不注册 Workflow 节点，因此不允许脚本调用。

## AI 调用

不传 root_id 时列出授权目录 ID，不泄露绝对目录路径。搜索示例：

```json
{
  "root_id": "project",
  "path": "ai-assistant/src",
  "pattern": "PlanWrite",
  "glob": "**/*.rs",
  "literal": true,
  "ignore_case": false,
  "output_mode": "content",
  "limit": 50
}
```

- path 为空表示整个授权根，使用 `/` 分隔的相对路径。
- literal 默认 true；false 使用 Rust 正则，不支持 PCRE 回溯引用、环视或跨行匹配。
- glob 相对于授权根，而非 path 子目录。
- content 默认返回相对路径、从 1 开始的行号、文本、line_truncated。
- files_with_matches 每个命中文件返回一次路径。
- count 统计每文件的匹配行数，不是匹配出现次数。
- limit 不得为零，超过宿主 max_results 时按宿主上限执行。

## 权限与结果完整性

拒绝绝对路径、上级目录、UNC、盘符和 Windows ADS 路径。跳过符号链接、Windows
重解析点；实际文件打开通过目录 capability 限制，避免链接替换竞争扩大读取范围。
这不等价于操作系统沙箱：不要授权含敏感硬链接、特殊文件或被恶意移动文件的敌对目录。

默认排除隐藏路径、二进制及非 UTF-8 文件，遵守逐级 `.gitignore` / `.ignore`；即使
没有 Git 仓库也生效。暂不读取全局 Git excludes 或 `.git/info/exclude`。AI 无法关闭
这些范围限制。目录迭代顺序不保证，不提供 offset 翻页或文件系统快照一致性。

返回 complete、truncated、reason 和 skipped_io / skipped_large / skipped_binary /
skipped_links 计数。IO 失败、大文件跳过和搜索预算耗尽会让 complete=false；明确排除的
隐藏、忽略、链接和二进制文件不属于搜索完整性的范围。单行截断单独用 line_truncated
标明。零匹配与非法正则、路径错误有区别，不得用不完整结果断言目标不存在。

结果 JSON 直接进入 to_ai，不经过 RPC 的 600 字摘要层。输出预算包含 JSON 外层，按
完整条目截断；不足时应缩小路径或 glob 重试。所有会话共享服务并发上限，繁忙返回 429。
超时返回 408 或带 reason 的不完整结果。取消会通知后台 worker；已阻塞的操作系统 IO
可能稍后才结束，期间仍占并发名额，不保证硬终止 IO。

实现使用 Rust regex/globset/ignore/cap-std，不执行 Shell，也不依赖机器安装 rg。
搜索内容是不可信数据，不能当作指令。调用走 Runtime 原有工具追踪，宿主应注意敏感
搜索结果的日志访问权限和保留策略。
