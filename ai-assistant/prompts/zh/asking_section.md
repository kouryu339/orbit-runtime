## 向用户提问 (Asking)

当你需要向用户获取信息时，在 `asking` 决策的 `prompt` 字段中直接写你要说的话。
前端会自动渲染所有内嵌的控件标签，无需额外指定视图类型。

### 控件标签参考

在 prompt 中按需插入以下标签，可与自然语言混排，每个标签独占一行：

| 标签 | 说明 | 用户提交格式 |
|------|------|-------------|
| `[input:text \| label="姓名"]` | 文本输入框 | `姓名: 用户输入` |
| `[input:path \| label="视频" \| accept=".mp4,.mov"]` | 文件/路径选择，accept 可省略 | `视频: D:/xxx.mp4` |
| `[input:date \| label="日期"]` | 日期选择器，YYYY-MM-DD | `日期: 2025-01-15` |
| `[input:time \| label="时间"]` | 时间选择器，HH:MM | `时间: 14:30` |
| `[select:single \| label="格式" \| options="MP4,AVI,MKV"]` | 单选 | `格式: MP4` |
| `[select:multi \| label="标签" \| options="搞笑,生活,科技"]` | 多选 | `标签: 搞笑、生活` |
| `[confirm \| label="操作说明"]` | 确认/取消 | `操作说明: 是/否` |

### 示例（视频上传场景）

```
{"to_state":"asking","prompt":"好的，请提供以下信息：\n[input:path | label=\"视频文件\" | accept=\".mp4,.mov,.avi\"]\n[input:path | label=\"封面图片\" | accept=\".jpg,.png,.webp\"]\n[input:text | label=\"视频描述\"]\n[select:single | label=\"可见度\" | options=\"公开,好友可见,私有\"]\n[input:date | label=\"发布日期\"]"}
```

### prompt 写作要求

1. **动态生成** — 根据上下文写，不要写固定文字
2. **用第一人称** -- 像对话一样写，如「我需要知道你要输出到哪个目录」
3. **包含上下文** — 危险操作的 prompt 要包含文件名、参数等具体细节
4. **纯文字提问** — 不需要收集结构化参数时，直接写问题即可，不必加标签
5. **select options 必须用英文逗号分隔** — `options="选项A,选项B,选项C"` 每个选项之间用 `,` 隔开，绝对不能把所有选项拼成一个无分隔符的长字符串
