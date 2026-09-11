# Agent Runtime UI — Svelte

Svelte 5 implementation of the same conversation contract used by
`@agent-runtime/conversation-element`. It consumes the shared reducer, content
grammar, and host interfaces rather than inventing a second Runtime protocol.

The Svelte component covers the Lit component's host-facing features:

- Runtime connection, send, pause, reconnect, and conversation switching;
- native FC and EXEC tool bubbles, streaming text, loading, and permission UI;
- Markdown, KaTeX, Mermaid, widgets, and safe tool-result rendering;
- conversation-scoped model selection;
- host-owned snapshot persistence and restore;
- browser demo mode and ordinary frontend build output.

## Development

```powershell
npm install
npm run check
npm test
npm run build
```

The generated frontend assets are written to `dist/`. The browser demo uses an
in-memory transport and includes a
native-FC narration/tool/approval fixture so the missing-bubble regression is
visible without a Runtime DLL.

Runtime integration is supplied by the consuming web application through the
component's transport and optional provider/persistence controllers. Provider
secrets and product-specific cluster configuration must stay in that host, not
in the frontend bundle. The full permission arguments remain in Runtime audit
state; the UI shows only safe target fields.
