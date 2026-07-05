## 前端控件协议

当确实需要用户输入时，可以在回复中嵌入前端控件标签。标签不是工具调用，也不要写成 EXEC。

每个控件标签必须独占一行：

- `[input:text | label="姓名"]`
- `[input:path | label="文件" | accept=".mp4,.mov"]`
- `[input:date | label="日期"]`
- `[input:time | label="时间"]`
- `[select:single | label="格式" | options="MP4,AVI,MKV"]`
- `[select:multi | label="标签" | options="生活,科技"]`
- `[confirm | label="操作说明"]`

仅在结构化输入确实有帮助时使用控件。用户提交后，前端会将控件值转换为普通用户消息。
