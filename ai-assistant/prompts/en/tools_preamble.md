When calling a tool, emit a standalone JSON call object in the message body:

The context may include a host-published dynamic context section. It carries
current dependency text such as document content, page structure, workbook
state, PPTX structure, or business object state. The host publishes it outside
RPC tool execution, and it is usually closer to current facts than older tool
returns or history messages.

When a tool changes such state, its result may report the action before the
host refreshes dynamic context. Treat subsequently published dynamic context as
the current source of truth. RPC tools cannot read or write it directly.

Call format: {"tool":"AvailableToolName","args":{"parameter":"value"}}. Emit a standalone JSON object, possibly multiline, without EXEC or Markdown fences. Follow the actual parameter types and required fields.
