## 工作流基本语法

工作流就是一个包含基本信息和 `logic` 的对象。`schema` 字段中的 `orbit.workflow.resource/v1` 只是格式版本标识，不是说明文档地址。下面是最小完整源文本，未定义或调用任何业务节点：

```text
{
  "schema": "orbit.workflow.resource/v1",
  "id": "example",
  "metadata": {"name": "返回输入", "description": "原样返回文字。", "extensions": {}},
  "state": "draft",
  "revision": 1,
  "logic": {
    "schema": "orbit.workflow.logic/v1",
    "inputs": {"text": {"type": "String", "description": "需要返回的文字。"}},
    "variables": {"done": {"type": "bool", "initial": false}},
    "steps": [{"id": 1, "set": "done", "value": true}],
    "returns": {
      "text": {"value": input.text},
      "done": {"value": var.done}
    }
  }
}
```

1. **提交**：将整份源文本作为 `source` 字符串提交给当前开放的注册或验证工具；外层请求仍是标准 JSON。`state` 为 draft/registered。新资源 revision 为 1，更新提交当前 revision，成功后由 Runtime 递增；草稿转正使用转正操作。
2. **声明**：inputs、variables、returns 都是以名称为键的对象。输入写 type，可带静态 default；变量必须先声明 type、静态 initial，set 只能写已声明变量；返回值写在 value 中。输入和返回可带 disablename、简短 description；变量可带 disablename，不写 description。metadata.extensions 保存按命名空间划分的宿主对象。
3. **值与引用**：只在节点 args 内的任意值和 `returns.<name>.value` 内允许裸表达式。`input.名称` 引用输入，`数字ID.输出名` 引用节点输出，`var.名称` 引用变量。Pure 调用写成 `已登记函数名(参数, ...)`，可嵌套。对象和数组可递归混用这些值；带引号的字符串始终是固定文字，数字、布尔、null 是常量。不要提交内部 expression/literal/array/object 包装。
4. **操作**：steps 和各分支／循环体是有序数组。每项有全局唯一、稳定的正安全整数 id，只选下表一种操作；不写 op。参数名及节点输出不能自行编造。

| 操作 | 字段形状（中文占位需替换） |
| --- | --- |
| 工具／节点调用 | call: 已登记名称字符串，args: 参数对象 |
| 条件 | if: 表达式，then: 节点数组，otherwise: 节点数组 |
| 循环 | for: 数组表达式，body: 节点数组 |
| 多分支 | switch: 表达式，cases: [{value: 固定值, body: 节点数组}]，default: 节点数组 |
| 赋值 | set: 已声明变量名字符串，value: 表达式 |

5. **控制字段的表达式**：if、for、switch 和 set.value 仍用结构化写法，不直接写裸表达式。输入 `{"input":"名称"}`，节点输出 `{"step":数字ID,"output":"输出名"}`，变量 `{"var":"名称"}`，Pure `{"fn":"已登记函数名","args":[表达式,...]}`；固定对象用 `{"literal":对象}`。这些形状中的名称均为占位，不是节点或函数定义。
6. **作用域**：循环不写 as，默认当前层 item/index；args 裸表达式用 var.item/var.index，控制表达式用 `{"local":"item"}`／`{"local":"index"}`。index 从 0 开始。只能引用当前或祖先作用域中已完成的节点，不能直接返回循环／分支内部节点输出。switch 只执行首个匹配 case，无匹配走 default，无 fallthrough。

Schema 用来校验结构，不能替代以上基本语法；顺序、作用域和工具契约由 Runtime 检查。具体节点、参数、输出及 Pure 函数只取运行时真实契约，工具契约可按需通过 ToolDescribe 查询。语法说明不提供节点实现，不授予权限，也不提供脚本工具；仅调用当前 Available Tools 中开放的资源操作。
