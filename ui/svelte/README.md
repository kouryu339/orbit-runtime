# Agent Runtime UI — Svelte

Svelte 5 implementation of the same conversation contract used by
`@agent-runtime/conversation-element`. It consumes the shared reducer, content
grammar, and host interfaces rather than inventing a second Runtime protocol.

The Svelte component covers the Lit component's host-facing features:

- Runtime connection, send, pause, reconnect, and conversation switching;
- native FC and EXEC tool bubbles, streaming text, loading, and permission UI;
- Markdown, KaTeX, Mermaid, widgets, and safe tool-result rendering;
- conversation-scoped model selection;
- provider reasoning effort, Responses summaries and Anthropic thinking budgets;
- selected/pasted images, ordered text/image parts and stable reference colors;
- live execution plans, gateway errors and terminal approval updates;
- host-owned snapshot persistence and restore;
- browser demo mode and ordinary frontend build output.

## Development

```powershell
npm install
npm run check
npm test
npm run check:styles
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

The defaults match Lit (`en-US`, `studio`, `comfortable`, system color scheme).
Use `locale="zh-CN"` for Chinese controls. The component exposes the shared public
API through `bind:this`, including `state`, and emits the same
`agent-conversation-*` events. It supports `header`, `empty-state`,
`composer-prefix`, and `composer-suffix` slots. Snapshot persistence stays in the
host: saving is a public method, not a default header action.

All common host/protocol/content/transport types and helpers are re-exported by
the Svelte entry point. `progressiveReveal`, `hideToolCalls` and `disclaimer` match
the Lit properties. `RichContent` also accepts `colorScheme`, `contentBaseDir`
and `contentMetadata`; the earlier `dark` and `baseDir` props remain available.

Conversation, content, plan and Mermaid styles are scoped to Svelte roots and
generated from Lit. After changing Lit styles, run
`node ui/sync-svelte-styles.mjs` from the repository root. `check:styles` rejects
stale generated CSS. The conversation imports its own styles, so hosts do not
need to import the demo's `src/styles.css`.

Tests use jsdom for Markdown sanitization and compare representative fixtures
against the real Lit renderer. Run `npm run build` in `ui/lit` first when editing
shared package exports, then run the Svelte checks above.

The composer layout, reasoning/permission settings and send/pause/continue
state machine match Lit. Pass `composerController` for per-conversation settings
and optional `transport.resume` for continue. Unsupported actions are disabled.
See [the composer host contract](../lit/README.md#composer-controls); the demo
implements those callbacks in memory, without changing Runtime DLL behavior.
