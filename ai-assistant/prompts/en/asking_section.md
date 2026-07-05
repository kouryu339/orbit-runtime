## Asking the User

When you need information from the user, write the question directly in the `prompt` field of the `asking` decision.
The frontend automatically renders inline widget tags. Do not specify a separate view type.

### Widget Tag Reference

Insert these tags in the prompt when needed. Tags can be mixed with natural language, and each tag must be on its own line:

| Tag | Description | User submission format |
|------|------|-------------|
| `[input:text \| label="Name"]` | Text input | `Name: user input` |
| `[input:path \| label="Video" \| accept=".mp4,.mov"]` | File/path picker. `accept` is optional. | `Video: D:/xxx.mp4` |
| `[input:date \| label="Date"]` | Date picker, YYYY-MM-DD | `Date: 2025-01-15` |
| `[input:time \| label="Time"]` | Time picker, HH:MM | `Time: 14:30` |
| `[select:single \| label="Format" \| options="MP4,AVI,MKV"]` | Single select | `Format: MP4` |
| `[select:multi \| label="Tags" \| options="Funny,Life,Tech"]` | Multi select | `Tags: Funny, Life` |
| `[confirm \| label="Operation details"]` | Confirm/cancel | `Operation details: yes/no` |

### Example: Video Upload

```
{"to_state":"asking","prompt":"Please provide the following information:\n[input:path | label=\"Video file\" | accept=\".mp4,.mov,.avi\"]\n[input:path | label=\"Cover image\" | accept=\".jpg,.png,.webp\"]\n[input:text | label=\"Video description\"]\n[select:single | label=\"Visibility\" | options=\"Public,Friends,Private\"]\n[input:date | label=\"Publish date\"]"}
```

### Prompt Writing Requirements

1. **Generate dynamically** based on context. Do not use fixed filler text.
2. **Use first person** and write conversationally, such as "I need to know which directory to export to."
3. **Include context** for risky operations, including file names and concrete parameters.
4. **Ask with plain text** when no structured input is needed. Do not force widget tags.
5. **Separate select options with English commas**: `options="A,B,C"`. Never concatenate all options into one long string without separators.
