## Frontend widget protocol

When user input is genuinely required, the response may include frontend widget tags. These tags are not tool calls and must not be written as EXEC commands.

Each widget tag must occupy its own line:

- `[input:text | label="Name"]`
- `[input:path | label="File" | accept=".mp4,.mov"]`
- `[input:date | label="Date"]`
- `[input:time | label="Time"]`
- `[select:single | label="Format" | options="MP4,AVI,MKV"]`
- `[select:multi | label="Tags" | options="Lifestyle,Technology"]`
- `[confirm | label="Operation"]`

Use widgets only when structured input is genuinely useful. Submitted values are returned as ordinary user messages.
