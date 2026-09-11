<script lang="ts">
  import { onMount } from 'svelte';

  export let source = '';
  export let dark = false;
  let container: HTMLDivElement;
  let error = '';
  let renderVersion = 0;

  async function renderDiagram() {
    if (!container || !source.trim()) return;
    const version = ++renderVersion;
    try {
      const {default: mermaid} = await import('mermaid');
      if (version !== renderVersion) return;
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        theme: dark ? 'dark' : 'neutral',
      });
      const id = `orbit-mermaid-${crypto.randomUUID().replaceAll('-', '')}`;
      const result = await mermaid.render(id, source);
      if (version !== renderVersion) return;
      container.innerHTML = result.svg;
      error = '';
    } catch (cause) {
      if (version !== renderVersion) return;
      container.textContent = '';
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  onMount(() => { void renderDiagram(); });
  $: source, dark, container && void renderDiagram();
</script>

<div class="diagram" bind:this={container}></div>
{#if error}<pre class="diagram-error">{error}</pre>{/if}
