<script lang="ts">
  import { createEventDispatcher, onDestroy } from 'svelte';
  import DOMPurify from 'dompurify';
  import './mermaid.css';

  export let source = '';
  export let dark = false;
  let error = '';
  let renderVersion = 0;
  let svg = '';
  const emit = createEventDispatcher<{
    diagnostic: { code: string; message: string };
  }>();

  async function renderDiagram() {
    const version = ++renderVersion;
    if (!source.trim()) {
      svg = '';
      error = '';
      return;
    }
    try {
      const { default: mermaid } = await import('mermaid');
      if (version !== renderVersion) return;
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        theme: dark ? 'dark' : 'neutral',
        flowchart: { htmlLabels: false },
      });
      const id = `orbit-mermaid-${crypto.randomUUID().replaceAll('-', '')}`;
      const result = await mermaid.render(id, source);
      if (version !== renderVersion) return;
      svg = DOMPurify.sanitize(result.svg, {
        USE_PROFILES: { svg: true, svgFilters: true },
        FORBID_TAGS: ['script', 'foreignObject', 'iframe', 'object', 'embed'],
        FORBID_ATTR: ['onload', 'onclick', 'onerror'],
      });
      error = '';
    } catch (cause) {
      if (version !== renderVersion) return;
      svg = '';
      error = cause instanceof Error ? cause.message : String(cause);
      emit('diagnostic', { code: 'mermaid-render-failed', message: error });
    }
  }

  onDestroy(() => {
    ++renderVersion;
  });
  $: (source, dark, void renderDiagram());
</script>

<div class="orbit-svelte-mermaid">
  {#if error}<pre part="mermaid-fallback"><code>{source}</code></pre>
  {:else if svg}<div class="diagram" part="mermaid-diagram">
      {@html svg}
    </div>{/if}
</div>
