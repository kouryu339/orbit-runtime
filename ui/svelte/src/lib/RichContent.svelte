<script lang="ts">
  import { createEventDispatcher, onDestroy } from 'svelte';
  import type { ConversationHostCapabilities } from '@agent-runtime/conversation-element/host';
  import {
    parseConversationContent,
    renderSafeMarkdown,
    toolCallForId,
    toolStatusLabel,
    toolDisplaySummary,
    revealBaseline,
    nextRevealLength,
    sourceLength,
    isSafeLink,
    type WidgetDefinition,
  } from '@agent-runtime/conversation-element/content';
  import type { ToolCallView } from '@agent-runtime/conversation-element/protocol';
  import Mermaid from './Mermaid.svelte';
  import './rich-content.css';
  export let content = '';
  export let toolCalls: ToolCallView[] = [];
  export let dark = false;
  export let colorScheme = 'light';
  export let hideToolCalls = false;
  export let widgetsExpired = false;
  export let capabilities: ConversationHostCapabilities = {};
  export let locale = 'en-US';
  export let baseDir: string | undefined = undefined;
  export let contentBaseDir: string | undefined = undefined;
  export let contentMetadata: Record<string, unknown> | undefined = undefined;
  export let reveal = false;
  const dispatch = createEventDispatcher<{
    submit: { content: string };
    complete: void;
    reveal: void;
    error: { message: string };
  }>();
  let visibleLength = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  let previousContent = '';
  let completedContent: string | undefined;
  let widgetValues: Record<string, string | string[]> = {};
  let otherValues: Record<string, string> = {};
  let otherEnabled = new Set<string>();
  let submitted = false;
  const widgetGroupId = crypto.randomUUID();
  let capabilityError = '';
  $: zh = locale.toLowerCase().startsWith('zh');
  $: effectiveDark = dark || colorScheme === 'dark';
  $: parts = parseConversationContent(content);
  $: widgets = parts.flatMap((part) =>
    part.kind === 'widgets' ? part.widgets : [],
  );
  $: canSubmit = hasValue(widgets, widgetValues, otherValues, otherEnabled);
  $: lastMarkdown = parts.map((part) => part.kind).lastIndexOf('markdown');
  $: updateReveal(content, reveal);
  let renderedParts: ReturnType<typeof parseConversationContent> = [];
  $: {
    let consumed = 0;
    renderedParts = parts.map((part, index) => {
      const available = Math.max(0, visibleLength - consumed);
      consumed += sourceLength(part);
      return part.kind === 'markdown' && reveal && index === lastMarkdown
        ? { ...part, content: part.content.slice(0, available) }
        : part;
    });
  }
  function complete() {
    if (completedContent === content) return;
    completedContent = content;
    queueMicrotask(() => dispatch('complete'));
  }
  function updateReveal(value: string, enabled: boolean) {
    clearInterval(timer);
    const baseline = revealBaseline(value);
    visibleLength =
      !previousContent || !value.startsWith(previousContent)
        ? baseline
        : Math.max(baseline, Math.min(visibleLength, value.length));
    previousContent = value;
    if (
      !enabled ||
      globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches
    ) {
      visibleLength = value.length;
      complete();
      return;
    }
    if (visibleLength >= value.length) {
      complete();
      return;
    }
    timer = setInterval(() => {
      const remaining = value.length - visibleLength;
      visibleLength = nextRevealLength(
        value,
        visibleLength,
        remaining > 200 ? 16 : remaining > 80 ? 8 : remaining > 24 ? 4 : 2,
      );
      dispatch('reveal');
      if (visibleLength >= value.length) {
        clearInterval(timer);
        complete();
      }
    }, 80);
  }
  onDestroy(() => clearInterval(timer));
  async function perform(action: () => void | Promise<unknown>) {
    try {
      await action();
    } catch (cause) {
      capabilityError = cause instanceof Error ? cause.message : String(cause);
      dispatch('error', { message: capabilityError });
    }
  }
  function markdownActions(node: HTMLElement) {
    const click = (event: MouseEvent) => {
      if (!(event.target instanceof Element)) return;
      const anchor = event.target.closest('a');
      if (anchor) {
        if (!isSafeLink(anchor.href)) {
          event.preventDefault();
          return;
        }
        if (capabilities.openLink) {
          event.preventDefault();
          void perform(() =>
            capabilities.openLink!({ url: anchor.href, source: 'markdown' }),
          );
        }
        return;
      }
      const image = event.target.closest('img');
      if (image && capabilities.openImage) {
        event.preventDefault();
        void perform(() =>
          capabilities.openImage!({
            source: image.getAttribute('src') ?? image.src,
            alt: image.alt,
            baseDir: baseDir ?? contentBaseDir,
            metadata: contentMetadata,
          }),
        );
      }
    };
    node.addEventListener('click', click);
    return {
      destroy() {
        node.removeEventListener('click', click);
      },
    };
  }
  function setOther(widget: WidgetDefinition, enabled: boolean) {
    otherEnabled = new Set(otherEnabled);
    if (enabled) otherEnabled.add(widget.raw);
    else otherEnabled.delete(widget.raw);
    if (enabled && widget.kind === 'select:single')
      widgetValues = { ...widgetValues, [widget.raw]: '' };
  }
  function answer(widget: WidgetDefinition): string | string[] {
    const value = widgetValues[widget.raw] ?? '';
    if (!otherEnabled.has(widget.raw))
      return typeof value === 'string' ? value.trim() : value;
    const other = (otherValues[widget.raw] ?? '').trim();
    return widget.kind === 'select:multi'
      ? [
          ...new Set([
            ...(Array.isArray(value) ? value : []),
            ...(other ? [other] : []),
          ]),
        ]
      : other;
  }
  function toggleMulti(
    widget: WidgetDefinition,
    option: string,
    checked: boolean,
  ) {
    const value = widgetValues[widget.raw];
    const values = Array.isArray(value) ? value : [];
    widgetValues = {
      ...widgetValues,
      [widget.raw]: checked
        ? [...new Set([...values, option])]
        : values.filter((v) => v !== option),
    };
  }
  function hasValue(
    items: WidgetDefinition[],
    _values = widgetValues,
    _others = otherValues,
    _enabled = otherEnabled,
  ) {
    return (
      !items.some(
        (w) => otherEnabled.has(w.raw) && !(otherValues[w.raw] ?? '').trim(),
      ) && items.some((w) => answer(w).length > 0)
    );
  }
  function submit() {
    if (widgetsExpired || submitted || !hasValue(widgets)) return;
    const content = widgets
      .map((w) => [w.label, answer(w)] as const)
      .filter(([, v]) => v.length > 0)
      .map(
        ([label, value]) =>
          `${label}: ${Array.isArray(value) ? value.join(', ') : value}`,
      )
      .join('\n');
    submitted = true;
    dispatch('submit', { content });
  }
  async function pickPath(widget: WidgetDefinition) {
    await perform(async () => {
      const result = await capabilities.pickPath?.({
        mode: 'file',
        label: widget.label,
        accept: widget.accept
          ?.split(',')
          .map((v) => v.trim())
          .filter(Boolean),
        multiple: false,
      });
      if (result?.paths[0])
        widgetValues = { ...widgetValues, [widget.raw]: result.paths[0] };
    });
  }
</script>

<div class="orbit-svelte-rich-content">
  {#each renderedParts as part}
    {#if part.kind === 'markdown'}
      <div class="part markdown" use:markdownActions>
        {@html renderSafeMarkdown(part.content)}
      </div>
    {:else if part.kind === 'mermaid'}<Mermaid
        source={part.source}
        dark={effectiveDark}
        on:diagnostic
      />
    {:else if part.kind === 'tool' && !hideToolCalls}
      {@const call = toolCallForId(toolCalls, part.callId)}
      <details
        class="part tool"
        data-status={call.status}
        open={call.status === 'waiting_permission'}
      >
        <summary
          ><span class="tool-dot"></span><span class="tool-title"
            >{call.title}</span
          ><span class="tool-state">{toolStatusLabel(call.status, locale)}</span
          ></summary
        >
        <div class="tool-detail">{toolDisplaySummary(call, locale)}</div>
      </details>
    {:else if part.kind === 'widgets'}
      <div
        class="part widgets"
        data-inactive={String(widgetsExpired || submitted)}
      >
        {#each part.widgets as widget}
          {@const disabled = widgetsExpired || submitted}
          {@const value = widgetValues[widget.raw] ?? ''}
          <div class="widget">
            <span class="widget-label">{widget.label}</span>
            {#if widget.kind === 'select:single' || widget.kind === 'select:multi'}
              {@const multiple = widget.kind === 'select:multi'}
              <div class="choice-list" role="group" aria-label={widget.label}>
                {#each [...new Set(widget.options)].filter((v) => !/^(其他|其它|other)$/i.test(v.trim())) as option}
                  <label class="choice-option"
                    ><input
                      type={multiple ? 'checkbox' : 'radio'}
                      name={`${widgetGroupId}:${encodeURIComponent(widget.raw)}`}
                      {disabled}
                      checked={(Array.isArray(value)
                        ? value
                        : [value]
                      ).includes(option) &&
                        (multiple || !otherEnabled.has(widget.raw))}
                      on:change={(event) => {
                        if (multiple)
                          toggleMulti(
                            widget,
                            option,
                            event.currentTarget.checked,
                          );
                        else {
                          setOther(widget, false);
                          widgetValues = {
                            ...widgetValues,
                            [widget.raw]: option,
                          };
                        }
                      }}
                    /><span>{option}</span></label
                  >
                {/each}
                <div class="choice-option other-option">
                  <input
                    class="other-toggle"
                    type={multiple ? 'checkbox' : 'radio'}
                    name={`${widgetGroupId}:${encodeURIComponent(widget.raw)}`}
                    {disabled}
                    checked={otherEnabled.has(widget.raw)}
                    aria-label={`${widget.label}: ${zh ? '其他' : 'Other'}`}
                    on:change={(e) => setOther(widget, e.currentTarget.checked)}
                  />
                  <input
                    class="other-input"
                    type="text"
                    {disabled}
                    aria-label={`${widget.label}: ${zh ? '其他内容' : 'Other answer'}`}
                    placeholder={zh
                      ? '其他，请输入…'
                      : 'Other, please specify…'}
                    value={otherValues[widget.raw] ?? ''}
                    on:focus={() => setOther(widget, true)}
                    on:input={(e) => {
                      setOther(widget, true);
                      otherValues = {
                        ...otherValues,
                        [widget.raw]: e.currentTarget.value,
                      };
                    }}
                  />
                </div>
              </div>
            {:else if widget.kind === 'confirm'}
              <div class="choice-row">
                <button
                  type="button"
                  {disabled}
                  aria-pressed={value === 'yes'}
                  on:click={() =>
                    (widgetValues = { ...widgetValues, [widget.raw]: 'yes' })}
                  >{zh ? '确认' : 'Confirm'}</button
                ><button
                  type="button"
                  {disabled}
                  aria-pressed={value === 'no'}
                  on:click={() =>
                    (widgetValues = { ...widgetValues, [widget.raw]: 'no' })}
                  >{zh ? '取消' : 'Cancel'}</button
                >
              </div>
            {:else if widget.kind === 'input:path'}
              <div class="path-row">
                <input
                  type="text"
                  aria-label={widget.label}
                  {disabled}
                  value={String(value)}
                  on:input={(e) =>
                    (widgetValues = {
                      ...widgetValues,
                      [widget.raw]: e.currentTarget.value,
                    })}
                />{#if capabilities.pickPath}<button
                    type="button"
                    {disabled}
                    on:click={() => void pickPath(widget)}
                    >{zh ? '浏览…' : 'Browse…'}</button
                  >{/if}
              </div>
            {:else}<input
                type="text"
                aria-label={widget.label}
                {disabled}
                value={String(value)}
                on:input={(e) =>
                  (widgetValues = {
                    ...widgetValues,
                    [widget.raw]: e.currentTarget.value,
                  })}
              />{/if}
          </div>
        {/each}
      </div>
    {/if}
  {/each}
  {#if widgets.length}
    {#if submitted || widgetsExpired}<div
        class="part widget-status"
        role="status"
      >
        {submitted
          ? zh
            ? '已提交'
            : 'Submitted'
          : zh
            ? '已过期 · 请在最新消息中回复'
            : 'Expired · Reply in the latest message'}
      </div>
    {:else}<div class="part widgets widget-actions">
        <button type="button" disabled={!canSubmit} on:click={submit}
          >{zh ? '提交' : 'Submit'}</button
        >
      </div>{/if}
  {/if}
  {#if capabilityError}<p role="alert">{capabilityError}</p>{/if}
</div>
