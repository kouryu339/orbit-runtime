# 结构化工作流示例

- [args 与 return 表达式示例](args-and-returns.logic.workflow)：使用内置节点，不依赖 Excel 工具。
- [完整资源源文档](excel-read-rows.resource.workflow)：元数据、草稿状态、revision 和逻辑。
- [Excel 逻辑源文档](excel-read-rows.logic.workflow)。
- [解析后的资源 JSON](excel-read-rows.resource.json)及[逻辑 JSON](excel-read-rows.logic.json)：供 Schema 校验和程序读取。
- [资源 Schema](../../corework/src/workflow/resource.schema.json)及[逻辑 Schema](../../corework/src/workflow/logic.schema.json)（Draft 2020-12）。

源文档沿用 JSON 外壳，只在节点 `args` 的参数值和 `returns.<name>.value` 中允许裸引用、Pure 函数调用。嵌套数组和对象的值也适用此规则。其他部分继续使用标准 JSON。本轮不扩展控制步骤语法；步骤 ID 由 AI 填写，文档内唯一，用于后续按 ID 读取和修改。

```text
"args": {
  "session_id": 1.session_id,
  "path": input.path,
  "count": add(var.count, 1),
  "names": split(5.value, ["，", ","]),
  "text": "add(var.count, 1)",
  "items": [var.item, "固定文字", 123]
}
```

带引号的值始终是字符串常量；裸引用和 Pure 调用由 Runtime 解析。无需 `=` 前缀或用户编写包装对象。Pure 支持嵌套，例如 `item(split(input.names, [","]), 0)`。语法解析不执行函数，运行时仅调用已登记的 Pure 节点。未声明变量、作用域外的 item／index、未完成或不存在的节点输出都会被拒绝。

源文档采用 `.workflow` 扩展名，不能直接交给标准 JSON 解析器；Runtime `parse_source` 将逻辑源码解析为普通 JSON 结构，`normalize_source` 可解析完整资源外壳用于 Schema 校验，但不导入或发布。内部 `expression`／`literal`／`array`／`object` 标记用于保留表达式与常量差异，不是源文档写法。

对外仅提供 `workflow.list/register/remove/validate/promote/execute`。注册与验证提交完整资源源文本 `source`；`names.resource.workflow` 仅使用内置节点，可用于本地验证。`metadata`、`state`、`revision` 均在源文档中，更新时提交当前 revision，成功后 Runtime 递增。旧脚本式入口已关闭。

示例打开 Excel，查询“是否执行”为空的行，逐行按 `column` 选择读取姓名或手机号。姓名分支将空字符串记录为提示，其他值按中英文逗号拆分后打印。未匹配的列名记录提示并继续下一行。这里空字符串比较不代表通用 Excel 空值判定，null 的行为取决于工具契约。

示例输入：

```json
{
  "path": "D:\\data\\申请表.xlsx",
  "sheet": "申请表",
  "column": "姓名"
}
```

格式规则：

- 用 `call`、`if`、`for`、`switch`、`set` 直接区分步骤，不再额外写 `op`。每个步骤只能声明一种操作。数字 ID 全局唯一且稳定，数组顺序决定执行顺序。
- args 和 return.value 中，引用写作 `input.name`、`数字ID.output`、`var.name`；旧结构化引用对象仍用于兼容接口。
- args 和 return.value 中可以直接写普通对象、数组，内部每个值按相同语法解析。
- switch 的 case 数量可变，只执行匹配分支；不匹配时执行 default，没有 fallthrough。
- 返回值使用顶层已完成步骤；此示例不从循环内部收集读取结果。
- `variables` 声明工作流变量的类型和静态初始值，`{"var":"processed_count"}` 读取当前值，`{"id":12,"set":"processed_count","value":1}` 显式修改。示例在每行遍历末尾累加 `processed_count`，包含未匹配列名的情况。
- `for` 指定数组，`body` 指定循环体；不再提供 `as` 别名。args 中通过 `var.item` 读取当前元素、`var.index` 读取从 0 开始的索引。嵌套循环引用当前层的 item／index，离开循环体后不可引用；仍编译为原有 ForEachNode。
- 变量每次运行独立初始化；分支和循环可更新它。变量名不能与输入重名，`item`、`index` 为旧编译后端保留名。表达式通过 var.name 引用变量，var.item／var.index 为循环局部引用。
- 第一版变量类型支持 `Any`、`String`、`bool`／`Boolean`、`i64`、`f64`、`Object` 和 `Array<T>`。初始值和直接字面量赋值检查类型；动态表达式继续沿用原有工具／节点契约，尚无全面的动态赋值类型证明。空数组初始值仍受旧后端限制。

输入、输出和变量的说明字段：

| 位置 | `disablename` | `description` |
| --- | --- | --- |
| `inputs.<name>` | 可选，给用户看的名称 | 给 AI 看的短句，说明用途和限制 |
| `returns.<name>` | 可选，给用户看的名称 | 给 AI 看的短句，说明用途和限制 |
| `variables.<name>` | 可选，给用户看的名称 | 不添加此字段，用途由逻辑表达 |

字段名沿用 `disablename`。显示名称不参与引用，引用仍使用原有标识。`description` 缺省时兼容旧逻辑；新文档应填写简短的用途和限制。

返回值增加 `value` 字段以容纳说明，例如：

```text
{
  "processed_count": {
    "value": var.processed_count,
    "disablename": "已遍历行数",
    "description": "本次遍历的行数；包含未匹配列名的情况。"
  }
}
```

旧的直接返回表达式仍可读取。编译时输入、输出说明映射到原始 `PinMetadata.display_name`／`description`；变量映射到新增的可选 `BlueprintVariable.display_name`。原始变量的旧 `description` 字段继续兼容，但新逻辑不新增变量描述。

资源层归属：

| 字段 | 维护者与用途 |
| --- | --- |
| `id` | 作者指定的稳定资源身份 |
| `metadata` | 名称、描述及按命名空间保存的宿主扩展 |
| `state` | Runtime 生命周期；`draft` 是草稿，`registered` 是已注册正式资源 |
| `revision` | Runtime 维护的资源版本 |
| `logic` | AI 和画布通过统一修改接口编辑的业务逻辑 |

Runtime 保存完整资源源文档并在启动时自动恢复草稿和正式资源。源文档是唯一业务事实来源，执行图在工具契约安装后重新编译。新源文件保存在管理目录的 `<id>.workflow.json`，带裸表达式时依然不能用标准 JSON 解析器直接读取。旧脚本及旧图文件不自动加载、不自动迁移；原文件保留。

移除将删除管理目录内对应源文件，作者备份请放在目录外。正式引用存在破坏性接口变化时拒绝更新；转正仍使用独立 promote 操作。

Excel 工具名称和字段沿用设计讨论中的契约，运行依赖宿主注册相应工具，输入输出字段和 session 生命周期以实际工具契约为准。本轮仅提供格式示例，未执行 Excel 操作。

Schema 检查结构；ID 唯一性、引用顺序、作用域、工具存在性及类型兼容性仍由 Runtime 检查。新增 switch case 只需增加 cases 元素，不需要修改 Schema。
