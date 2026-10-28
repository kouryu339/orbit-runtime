<script lang="ts">
  import {
    displayText,
    type LedgerRecord,
  } from '@agent-runtime/conversation-element/protocol';
  import { referenceColor } from '@agent-runtime/conversation-element/content';
  import type { ConversationHostCapabilities } from '@agent-runtime/conversation-element/host';
  import ImageReference from './ImageReference.svelte';
  export let record: LedgerRecord;
  export let capabilities: ConversationHostCapabilities = {};
  export let conversationId: string | undefined = undefined;
  export let locale = 'en-US';
  $: raw = record.metadata?.extra?.parts;
  $: parts = Array.isArray(raw)
    ? raw.filter(
        (
          v,
        ): v is
          | { type: 'text'; text: string }
          | { type: 'image'; image_id: string } =>
          Boolean(
            v &&
              typeof v === 'object' &&
              ((v.type === 'text' && typeof v.text === 'string') ||
                (v.type === 'image' && typeof v.image_id === 'string')),
          ),
      )
    : [];
  $: message = displayText(record);
  $: error = record.metadata?.extra?.pending_error;
  function mentions(value: string) {
    let offset = 0;
    return value.split(/(@[\p{L}\p{N}_-]+)/gu).map((text) => {
      const previous = value[offset - 1] ?? '';
      offset += text.length;
      return {
        text,
        imageId: undefined as string | undefined,
        key:
          text.startsWith('@') &&
          text.length > 1 &&
          !/[A-Za-z0-9._%+-]/.test(previous)
            ? `mention:${text.slice(1).toLocaleLowerCase()}`
            : null,
      };
    });
  }
  $: tokens = !parts.length
    ? mentions(message)
    : [
        ...(!parts.some((p) => p.type === 'text') &&
        message !== '[Image]' &&
        !error
          ? mentions(message)
          : []),
        ...parts.flatMap((part, index) =>
          part.type === 'text'
            ? mentions(part.text)
            : [
                {
                  text: `@图片${parts.slice(0, index + 1).filter((p) => p.type === 'image').length}`,
                  key: `image:${part.image_id}`,
                  imageId: part.image_id,
                },
              ],
        ),
      ];
</script>

{#each tokens as token}{#if token.imageId}<span
      data-reference-key={token.key}
      style={`--reference-color: ${referenceColor(token.key!)}`}
      ><ImageReference
        imageId={token.imageId}
        {conversationId}
        {capabilities}
        fallback={token.text}
        label={locale.startsWith('zh')
          ? `预览${token.text.slice(1)}`
          : `Preview ${token.text}`}
        on:error
      /></span
    >
  {:else if token.key}<span
      class="at-reference"
      style={`--reference-color: ${referenceColor(token.key)}`}
      data-reference-key={token.key}>{token.text}</span
    >{:else}{token.text}{/if}{/each}
{#if error}<br />{String(error)}{/if}
