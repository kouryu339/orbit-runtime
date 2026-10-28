# Orbit Conversation — Svelte frontend

Use `orbit-conversation-svelte.js` as a browser ES module. It includes the Svelte
runtime, component styles, Markdown, math, diagram rendering, and fonts. No Svelte
compiler, Lit element, npm install, or CDN is required by the consuming page.
Serve the file over HTTP(S) or your desktop app's asset protocol; do not open it
as a standalone HTML page. The host still supplies a ConversationTransport.

```html
<div id="conversation" style="height: 100dvh"></div>
<script type="module">
  import { mountConversation } from './orbit-conversation-svelte.js';
  // Obtain transport from your host integration; this is not a demo transport.
  const conversation = mountConversation(document.querySelector('#conversation'), {
    transport: window.conversationTransport,
    locale: 'zh-CN',
    colorScheme: 'light',
  });
  // conversation.api.send(text), pause(), resume(), and other public component methods.
  // await conversation.destroy() when removing the view.
</script>
```

Exports include `Conversation`, `RichContent`, `ProviderEditor`, `PlanCard`,
`mountConversation`, and shared host/protocol/transport runtime utilities.
Source projects and TypeScript contracts are in `ui/svelte` and `ui/lit` at this
release's Git tag. A Svelte project can use the source package directly instead.

Styles are installed once in the document head when the module is imported.
The bundle targets modern browsers (ES2022); hosts with a Content Security Policy
must permit its injected stylesheet and embedded font data URLs.

`manifest.json` records the matching Runtime release, entry point, source commit,
and file SHA-256 values. Keep `LICENSE`, `NOTICE`, and `THIRD_PARTY_NOTICES.txt`
when redistributing the frontend. The separately attached versioned `.js` is
byte-for-byte identical to this archive's stable entry file.
