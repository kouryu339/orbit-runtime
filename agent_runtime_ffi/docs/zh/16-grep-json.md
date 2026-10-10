# GrepJson：复用 Grep 授权的结构化 JSON 读取

Grep 和 GrepJson 同时提供，分别处理普通文本和 JSON 结构。两者复用现有 resources.json 的 grep 配置、GrepService、授权目录 capability、超时和并发名额；没有 grep_json 配置，也不需要新增 FFI 注册接口。

默认 thinking / thinking-pro 不开放这两个工具。宿主在自己的 Skill tools 中显式选择 Grep、GrepJson 或两者。两者仅供 AI 调用，不注册 Workflow 节点。工具可见性不代表文件权限；未授权 roots 时无法读取文件。

## 读取配置

沿用 grep.roots。现有默认 max_file_bytes 为 2 MiB；更大的 JSON 必须由宿主提高该上限，例如 32 MiB，并按实际内存预算调整 max_entries。AI 无法提高宿主上限。

| 配置 | GrepJson 用途 |
|---|---|
| roots | 与 Grep 完全相同的可读根目录 |
| max_file_bytes | 原始 JSON 文件字节上限，也用于 ignore 文件读取 |
| max_output_bytes | 包含元数据在内的完整返回 JSON 上限 |
| max_results | 每页结果、子节点或字符串字符数上限 |
| max_entries | JSON 值节点预算；对象、数组和每个值各计一次 |
| max_pattern_bytes | 路径、Pointer、查询值、版本等输入长度上限 |
| timeout_ms / max_concurrent_searches | Grep 和 GrepJson 共用并发名额，分别执行超时控制 |

max_line_bytes 不用于截断 JSON；max_regex_bytes 不适用，GrepJson 不执行正则。隐藏、ignored、符号链接和 Windows reparse point 路径仍拒绝。实际打开受目录 capability 限制。

## 输入与输出

- root_id、path：授权根 ID 和相对 JSON 文件路径。省略 root_id 列出授权 ID。
- action：outline（默认）、search、read、page。
- pointer：标准 JSON Pointer，空字符串表示根；字段中的 ~ 和 / 分别写为 ~0、~1。
- search 的 field 为精确字段名，value_json 为序列化 JSON 值，contains 为字符串字面子串；同时提供时为 AND。不会跨对象拼接条件。
- offset / limit：从 0 开始的匹配位置、子节点位置或字符串 Unicode scalar 位置。非零 offset 必须提供 expected_revision，继续时保持原查询条件和 pointer。
- expected_revision：上一份结果的版本；包括 offset=0 的后续读取也建议提供。

所有文件结果返回 revision（sha256: 加原始文件字节的 SHA-256）、root_id、path、pointer、action、complete。

complete 只说明这次查询或子节点枚举是否到末尾；value_complete 才说明该 value 是否为完整原值。outline 和超限子树返回 descriptor，包含准确 pointer、类型、子节点数量，长字符串只提供显式 preview。不能用 preview 替换原值。

read 在预算内返回完整 value；超限自动返回可进一步读取的子节点描述。page 支持数组、对象以及长字符串。小条目放不下当前页的剩余空间时，下一页继续返回完整值，避免将页尾条目降为描述后再单独读取；单项本身超限才返回可向下读取的描述。对象按解析后的稳定键顺序分页，不依赖源文件缩进；字符串以 chunk 返回，按 offset 顺序拼接即可重建原字符串。complete=false 时返回 next_offset，继续携带 revision。若连一个描述项都无法容纳则明确报错，要求宿主提高输出预算，避免无进展翻页。

search 返回命中 Pointer、parent_pointer 和最近三个祖先的描述；祖先中的 id/name/type 仅作为有完整性标记的上下文线索。所有祖先仍可根据 Pointer 继续读取，返回结果不会声称这些线索是完整父对象。

支持任意精度 JSON 数字；合法字段名始终作为字段保留，包括与序列化库内部标记同名的字段。容器校验和建树在一次遍历中完成，字符串与数字交给序列化库解析，搜索 value_json 使用相同的保真解析路径。嵌套容器最多 128 层。重复对象键、非法 JSON、无效 Pointer、文件过大和节点预算超限均报错，不把不完整读取当作“没有找到”。仅支持标准 JSON，不自动修复 JSONC、JSONL 或损坏文件。

## 示例

先搜索：
```json
{"root_id":"project","path":"scene.json","action":"search","field":"id","value_json":"\"camera-01\"","limit":10}
```

得到 /sequences/0/bindings/4/id 和版本后，读取父对象：
```json
{"root_id":"project","path":"scene.json","action":"read","pointer":"/sequences/0/bindings/4","expected_revision":"sha256:上一结果的完整哈希"}
```

如果 tracks 太大，再读取它的数组分页：
```json
{"root_id":"project","path":"scene.json","action":"page","pointer":"/sequences/0/bindings/4/tracks","offset":0,"limit":20,"expected_revision":"sha256:上一结果的完整哈希"}
```

文件任意字节改变后，携带旧版本的读取返回 409 revision_conflict。没有服务端长期快照；每次调用重新读取并验证文件。重排数组后旧 Pointer 不可继续用于写入。

## 宿主修改契约

GrepJson 只读。宿主现有修改工具应接收文件引用、expected_revision、JSON Pointer 和修改值；需要替换对象时先确认读到 value_complete=true。建议同时传预期旧值，提供额外定位校验。

宿主必须在同一受保护写入过程内：读取文件原始字节并校验 SHA-256 → 解析并应用修改 → 执行业务 Schema 校验 → 原子落盘 → 返回新 revision。版本冲突时不写入，要求 AI 重新读取。

Rust 可复用 ai_assistant::systems::grep_json::document_revision；其他语言以 SHA-256 对文件原始字节计算，输出小写十六进制，前缀 sha256:。不能先格式化 JSON 再哈希。

锁必须覆盖版本比较到落盘，并协调所有写入者；只有校验步骤或只有临时文件重命名都不能防止并发覆盖。GrepJson 本身不替宿主提供写入事务或 Schema 校验。

文件读取会检查前后长度和修改时间以识别常见并发写入；这不构成跨调用快照，也不能保证面对不受协调的原地写入者时的一致视图。revision 表示实际读取的字节，后续版本校验仍必须执行。

## 资源与追踪

文件有界读取、节点与深度有界解析、完整条目输出；每次调用重新解析，不长期缓存大文档。超时或取消会通知后台 worker，阻塞中的系统 IO 可能稍后退出，名额在 worker 结束时才释放。调用沿用 Runtime 工具追踪，结果直接进入 to_ai，不经过 RPC 摘要截断。

## 验收结果

专项测试覆盖 19 种情况，另有 2 个通过真实工具执行上下文的验收用例。特殊合法字段与真实大整数同时保留，结构化 result 和 to_ai JSON 均验证。

长文档用例为 2,360,802 字节：先按 ID 定位摄像机，再按 Pointer 和同一 revision 完整读取 4 条轨道的 256 个关键帧，逐个比对全部字段。分页相关调用由 12 次降为 8 次；数组重排后旧 revision 返回 409，重新搜索定位到新位置。

这些用例验证 Runtime 的读取、分页和版本冲突。宿主修改工具的写入锁、业务 Schema 校验和并发修改仍须由宿主接入后验收；测试没有替代该职责。

## Engineering Review

Architecture: GrepJson 复用 Grep 的宿主授权及服务；写入由宿主承担。
Code: 一个工具提供 outline/search/read/page；通用 JSON Pointer 定位。
Concurrency: 共享并发限制，分页校验文件版本；写入锁由宿主负责。
Resource: 文件、节点、输出、查询长度和执行时间均受现有宿主配置限制。
Audit: 标明 revision、完整性、继续读取位置和明确错误。
Verification: 覆盖长数组、Unicode 长字符串、大整数与高精度数字、特殊合法字段、重复键、解析语法与深度、授权、取消、版本冲突与共享并发。
Risks: 超过宿主限制需调整配置；不支持非标准 JSON；宿主必须实现受保护的版本比较与写入。
Decisions Needed: 无。

宿主可选能力 Skill 示例（由宿主激活）：

```markdown
---
name: structured-file-reading
description: Read authorized text and structured JSON files.
tools: ["Grep", "GrepJson"]
---
Use Grep for text and GrepJson for JSON. Follow next_offset with expected_revision.
Read complete values before passing edits and the revision to the host editing tool.
```
