<script lang="ts">
  import { createEventDispatcher, onDestroy } from 'svelte';
  import type { ConversationHostCapabilities } from '@agent-runtime/conversation-element/host';
  import {
    parseConversationContent,
    renderSafeMarkdown,
    type WidgetDefinition,
  } from '@agent-runtime/conversation-element/content';
  import type { ToolCallView } from '@agent-runtime/conversation-element/protocol';
  import Mermaid from './Mermaid.svelte';

  export let content = '';
  export let toolCalls: ToolCallView[] = [];
  export let dark = false;
  export let hideToolCalls = false;
  export let widgetsExpired = false;
  export let capabilities: ConversationHostCapabilities = {};
  export let locale = 'zh-CN';
  export let baseDir = '';
  export let reveal = false;
  let revealed = '';
  let timer: ReturnType<typeof setInterval> | undefined;
  let previousContent = '';
  let capabilityError = '';

  const dispatch = createEventDispatcher<{ submit: { content: string }; complete: void }>();
  let widgetValues: Record<string, string | string[] | boolean> = {};
  let submitted = false;
  $: updateReveal(content, reveal);
  $: parts = parseConversationContent(revealed);
  function updateReveal(value: string, enabled: boolean) {
    clearInterval(timer);
    if (!enabled || globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
      revealed = value; previousContent = value;
      dispatch('complete'); return;
    }
    if (!value.startsWith(previousContent)) revealed = '';
    previousContent = value;
    timer = setInterval(() => {
      revealed = value.slice(0, revealed.length + Math.max(3, Math.ceil(value.length / 100)));
      if (revealed.length >= value.length) { clearInterval(timer); dispatch('complete'); }
    }, 16);
  }
  onDestroy(() => clearInterval(timer));

  function markdownActions(node: HTMLElement) {
    const click = async (event: MouseEvent) => {
      const target = event.target as HTMLElement;
      const anchor = target.closest('a');
      const image = target.closest('img');
      try {
        if (anchor && capabilities.openLink) { event.preventDefault(); await capabilities.openLink({url:anchor.href, source:'markdown'}); }
        else if (image && capabilities.openImage) { event.preventDefault(); await capabilities.openImage({source:image.src, alt:image.alt, baseDir}); }
      } catch (error) { capabilityError = String(error); }
    };
    node.addEventListener('click', click);
    return {destroy() { node.removeEventListener('click', click); }};
  }
  async function pickPath(widget: WidgetDefinition, key: string) {
    try {
      const result = await capabilities.pickPath?.({mode:widget.accept === 'directory' ? 'directory' : 'file', label:widget.label, accept:widget.accept ? [widget.accept] : undefined});
      if (result) widgetValues = {...widgetValues, [key]:result.paths.join('\n')};
    } catch (error) { capabilityError = String(error); }
  }

  function callFor(id: string): ToolCallView {
    return toolCalls.find((call) => call.id === id) ?? {
      id,
      title: 'Preparing tool call',
      status: 'placeholder',
      detail: '',
      toolName: '',
    };
  }

  function widgetKey(widget: WidgetDefinition, index: number): string {
    return `${widget.kind}:${widget.label}:${index}`;
  }

  function submitWidgets(widgets: WidgetDefinition[]) {
    const lines = widgets.map((widget, index) => {
      const key = widgetKey(widget, index);
      const value = widgetValues[key];
      const rendered = Array.isArray(value) ? value.join(', ') : String(value ?? '');
      return `${widget.label}: ${rendered}`;
    });
    submitted = true;
    dispatch('submit', { content: lines.join('\n') });
  }
</script>

{#each parts as part}
  {#if part.kind === 'markdown'}
    <div class="markdown" use:markdownActions>{@html renderSafeMarkdown(part.content)}</div>
    {#if capabilities.copyText}<button class="quiet copy" on:click={() => capabilities.copyText?.(part.content)}>{locale.startsWith('zh') ? '复制' : 'Copy'}</button>{/if}
  {:else if part.kind === 'mermaid'}
    <Mermaid source={part.source} {dark} />
  {:else if part.kind === 'tool' && !hideToolCalls}
    {@const call = callFor(part.callId)}
    <details class="tool" data-status={call.status} open={call.status === 'waiting_permission'}>
      <summary><span class="tool-dot"></span>{call.title}</summary>
      {#if call.detail}<pre>{call.detail}</pre>{/if}
    </details>
  {:else if part.kind === 'widgets'}
    <section class="widgets">
      {#each part.widgets as widget, index}
        {@const key = widgetKey(widget, index)}
        <label>
          <span>{widget.label}</span>
          {#if widget.kind === 'select:single'}
            <select bind:value={widgetValues[key]} disabled={widgetsExpired || submitted}>
              <option value="">请选择</option>
              {#each widget.options as option}<option value={option}>{option}</option>{/each}
            </select>
          {:else if widget.kind === 'select:multi'}
            <select multiple bind:value={widgetValues[key]} disabled={widgetsExpired || submitted}>
              {#each widget.options as option}<option value={option}>{option}</option>{/each}
            </select>
          {:else if widget.kind === 'confirm'}
            <input
              type="checkbox"
              checked={widgetValues[key] === true}
              disabled={widgetsExpired || submitted}
              on:change={(event) => { widgetValues[key] = event.currentTarget.checked; }}
            />
          {:else if widget.kind === 'input:date'}
            <input type="date" bind:value={widgetValues[key]} disabled={widgetsExpired || submitted} />
          {:else if widget.kind === 'input:time'}
            <input type="time" bind:value={widgetValues[key]} disabled={widgetsExpired || submitted} />
          {:else}
            <input bind:value={widgetValues[key]} disabled={widgetsExpired || submitted} />
            {#if widget.kind === 'input:path' && capabilities.pickPath}<button disabled={widgetsExpired || submitted} on:click={() => void pickPath(widget,key)}>选择路径</button>{/if}
          {/if}
        </label>
      {/each}
      <button type="button" disabled={widgetsExpired || submitted} on:click={() => submitWidgets(part.widgets)}>
        {submitted ? '已提交' : '提交'}
      </button>
    </section>
  {/if}
{/each}
{#if capabilityError}<p role="alert" class="error">{capabilityError}</p>{/if}
