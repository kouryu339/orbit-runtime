<script lang="ts">
  import { createEventDispatcher, onDestroy } from 'svelte';
  import type { ConversationHostCapabilities } from '@agent-runtime/conversation-element/host';
  import { isSafeImageSource } from '@agent-runtime/conversation-element/content';
  import './image-reference.css';
  export let imageId: string;
  export let conversationId: string | undefined = undefined;
  export let capabilities: ConversationHostCapabilities = {};
  export let label = 'Image';
  export let fallback = 'Image';
  let source = '';
  let alt = '';
  let failed = false;
  let controller: AbortController | undefined;
  const emit = createEventDispatcher<{
    error: { code: string; message: string; recoverable: boolean };
  }>();
  function report(error: unknown) {
    emit('error', {
      code: 'image-preview-failed',
      message: error instanceof Error ? error.message : String(error),
      recoverable: true,
    });
  }
  async function load(
    id: string,
    conversation: string | undefined,
    host: ConversationHostCapabilities,
  ) {
    controller?.abort();
    const pending = new AbortController();
    controller = pending;
    source = '';
    alt = '';
    failed = false;
    try {
      const result = await host.resolveImage?.({
        imageId: id,
        conversationId: conversation,
        signal: pending.signal,
      });
      if (pending.signal.aborted) return;
      if (result && isSafeImageSource(result.source)) {
        source = result.source;
        alt = result.alt ?? '';
      }
    } catch (error) {
      if (!pending.signal.aborted) report(error);
    }
  }
  $: load(imageId, conversationId, capabilities);
  onDestroy(() => controller?.abort());
  async function open() {
    try {
      await capabilities.openImage?.({
        source,
        alt: alt || label,
        metadata: { imageId },
      });
    } catch (error) {
      report(error);
    }
  }
</script>

<span class="orbit-svelte-image-reference">
  {#if source && !failed}<button
      type="button"
      aria-label={alt || label}
      title={alt || label}
      disabled={!capabilities.openImage}
      on:click={() => void open()}
      ><img
        src={source}
        alt={alt || label}
        on:error={() => (failed = true)}
      /></button
    >
  {:else}<span class="fallback">{fallback}</span>{/if}
</span>
