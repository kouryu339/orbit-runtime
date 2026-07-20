import { LitElement, css, html } from 'lit';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';
import {
  parseConversationContent,
  toolCallForId,
  type WidgetDefinition,
} from '../content/parser.js';
import { renderSafeMarkdown } from '../content/markdown.js';
import type { ConversationHostCapabilities } from '../host/types.js';
import type { ToolCallView } from '../protocol/types.js';
import './mermaid-diagram.js';

export class ConversationRichContentElement extends LitElement {
  static properties = {
    content: { type: String },
    toolCalls: { attribute: false },
    colorScheme: { type: String, attribute: 'color-scheme' },
    capabilities: { attribute: false },
    contentBaseDir: { attribute: false },
    contentMetadata: { attribute: false },
    hideToolCalls: { type: Boolean, attribute: 'hide-tool-calls' },
    locale: { type: String },
    reveal: { type: Boolean },
    widgetsExpired: { type: Boolean, attribute: 'widgets-expired' },
    visibleLength: { state: true },
    expandedTools: { state: true },
    widgetValues: { state: true },
    widgetSubmitted: { state: true },
  };

  static styles = css`
    :host {
      display: block;
      min-width: 0;
      color: var(--conversation-text);
    }

    .markdown {
      font: 400 14px/1.68 var(--conversation-font-body);
      overflow-wrap: anywhere;
    }

    .markdown > :first-child {
      margin-top: 0;
    }

    .markdown > :last-child {
      margin-bottom: 0;
    }

    .markdown h1,
    .markdown h2,
    .markdown h3 {
      margin: 1.15em 0 0.45em;
      color: var(--conversation-text-strong);
      font-family: var(--conversation-font-display);
      line-height: 1.25;
    }

    .markdown h1 { font-size: 18px; }
    .markdown h2 { font-size: 16px; }
    .markdown h3 { font-size: 14px; }
    .markdown p { margin: 0.72em 0; }
    .markdown ul, .markdown ol { padding-left: 22px; }
    .markdown blockquote {
      margin: 12px 0;
      padding: 2px 0 2px 13px;
      border-left: 2px solid var(--conversation-accent);
      color: var(--conversation-text-muted);
    }
    .markdown pre {
      overflow: auto;
      padding: 13px;
      border: 1px solid var(--conversation-border);
      border-radius: 10px;
      background: var(--conversation-code-background);
      font: 12px/1.55 var(--conversation-font-mono);
    }
    .markdown code {
      padding: 0.15em 0.35em;
      border-radius: 5px;
      background: var(--conversation-code-background);
      font-family: var(--conversation-font-mono);
    }
    .markdown pre code { padding: 0; background: transparent; }
    .markdown a { color: var(--conversation-accent); }
    .markdown table {
      width: 100%;
      max-width: 100%;
      table-layout: fixed;
      border-collapse: collapse;
      font-size: 13px;
    }
    .markdown th, .markdown td {
      padding: 7px 9px;
      border: 1px solid var(--conversation-border);
      text-align: left;
      vertical-align: top;
      white-space: normal;
      overflow-wrap: anywhere;
      word-break: break-word;
    }
    .markdown math[display="block"] {
      display: block;
      overflow-x: auto;
      margin: 14px 0;
      text-align: center;
    }

    .part + .part {
      margin-top: 12px;
    }

    .tool {
      border: 1px solid var(--conversation-border);
      border-radius: 10px;
      background: var(--conversation-surface-raised);
      color: var(--conversation-text);
    }

    .tool summary {
      display: flex;
      align-items: center;
      gap: 9px;
      padding: 9px 11px;
      cursor: pointer;
      list-style: none;
      font: 600 12px/1.4 var(--conversation-font-body);
    }

    .tool summary::-webkit-details-marker { display: none; }
    .tool-dot {
      width: 7px;
      height: 7px;
      border-radius: 50%;
      background: var(--conversation-tool-running);
      box-shadow: 0 0 0 3px color-mix(in srgb, var(--conversation-tool-running) 16%, transparent);
    }
    .tool[data-status="finished"] .tool-dot { background: var(--conversation-tool-success); }
    .tool[data-status="failed"] .tool-dot { background: var(--conversation-tool-error); }
    .tool[data-status="waiting_permission"] {
      border-color: color-mix(in srgb, var(--conversation-accent) 45%, var(--conversation-border));
    }
    .tool[data-status="waiting_permission"] .tool-dot {
      background: var(--conversation-accent);
      box-shadow: 0 0 0 3px color-mix(in srgb, var(--conversation-accent) 18%, transparent);
    }
    .tool-detail {
      margin: 0;
      padding: 0 11px 11px 27px;
      color: var(--conversation-text-muted);
      font: 12px/1.55 var(--conversation-font-mono);
      white-space: pre-wrap;
    }
    .widgets {
      display: grid;
      gap: 9px;
      padding: 12px;
      border: 1px solid var(--conversation-border);
      border-radius: 12px;
      background: var(--conversation-surface-raised);
    }
    .widget {
      display: grid;
      gap: 6px;
    }
    .widget-label {
      color: var(--conversation-text-muted);
      font: 600 11px/1.4 var(--conversation-font-body);
      letter-spacing: 0.03em;
    }
    input, select {
      min-width: 0;
      height: 34px;
      padding: 0 9px;
      border: 1px solid var(--conversation-border);
      border-radius: 8px;
      background: var(--conversation-surface);
      color: var(--conversation-text);
      font: 13px var(--conversation-font-body);
    }
    button {
      min-height: 34px;
      border: 0;
      border-radius: 8px;
      background: var(--conversation-accent);
      color: var(--conversation-accent-contrast);
      font: 650 12px var(--conversation-font-body);
      cursor: pointer;
    }
    button:disabled, input:disabled, select:disabled { opacity: 0.5; cursor: not-allowed; }
    .choice-row { display: flex; flex-wrap: wrap; gap: 7px; }
    .path-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 7px; }
    .choice-row button {
      border: 1px solid var(--conversation-border);
      background: var(--conversation-surface);
      color: var(--conversation-text);
    }
  `;

  declare content: string;
  declare toolCalls: ToolCallView[];
  declare colorScheme: string;
  declare capabilities: ConversationHostCapabilities;
  declare contentBaseDir: string | undefined;
  declare contentMetadata: Record<string, unknown> | undefined;
  declare hideToolCalls: boolean;
  declare locale: string;
  declare reveal: boolean;
  declare widgetsExpired: boolean;
  declare private visibleLength: number;
  declare private expandedTools: Set<string>;
  declare private widgetValues: Record<string, string | string[]>;
  declare private widgetSubmitted: boolean;
  private revealTimer: ReturnType<typeof globalThis.setInterval> | null = null;
  private completedContent = '';

  constructor() {
    super();
    this.content = '';
    this.toolCalls = [];
    this.colorScheme = 'light';
    this.capabilities = {};
    this.contentBaseDir = undefined;
    this.contentMetadata = undefined;
    this.hideToolCalls = false;
    this.locale = 'en-US';
    this.reveal = false;
    this.widgetsExpired = false;
    this.visibleLength = 0;
    this.expandedTools = new Set();
    this.widgetValues = {};
    this.widgetSubmitted = false;
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    this.stopReveal();
  }

  protected override willUpdate(changed: Map<PropertyKey, unknown>): void {
    if (!changed.has('content') && !changed.has('reveal')) return;
    if (!this.reveal) return;
    const previous = String(changed.get('content') ?? '');
    const baseline = revealBaseline(this.content);
    if (!previous || !this.content.startsWith(previous)) {
      this.visibleLength = baseline;
    } else {
      this.visibleLength = Math.max(this.visibleLength, baseline);
    }
    if (this.visibleLength > this.content.length) {
      this.visibleLength = this.content.length;
    }
  }

  protected override updated(changed: Map<PropertyKey, unknown>): void {
    if (!changed.has('content') && !changed.has('reveal')) return;
    if (!this.reveal) {
      this.stopReveal();
      queueMicrotask(() => this.emitRevealComplete());
      return;
    }
    if (this.visibleLength < this.content.length) this.startReveal();
    else this.emitRevealComplete();
  }

  protected override render() {
    const parts = parseConversationContent(this.content);
    let lastMarkdownIndex = -1;
    for (let index = parts.length - 1; index >= 0; index -= 1) {
      if (parts[index].kind === 'markdown') {
        lastMarkdownIndex = index;
        break;
      }
    }
    let consumed = 0;
    return html`${parts.map((part, partIndex) => {
      if (part.kind === 'markdown') {
        let content = part.content;
        if (this.reveal && partIndex === lastMarkdownIndex) {
          const available = Math.max(0, this.visibleLength - consumed);
          content = part.content.slice(0, available);
        }
        consumed += part.content.length;
        return html`<div class="part markdown" part="markdown">
          <div @click=${this.onMarkdownClick}>
            ${unsafeHTML(renderSafeMarkdown(content))}
          </div>
        </div>`;
      }
      consumed += sourceLength(part);
      if (part.kind === 'mermaid') {
        return html`<agent-mermaid-diagram
          class="part"
          .source=${part.source}
          .colorScheme=${this.colorScheme}
        ></agent-mermaid-diagram>`;
      }
      if (part.kind === 'tool') {
        if (this.hideToolCalls) return null;
        const call = toolCallForId(this.toolCalls, part.callId);
        return html`<details
          class="part tool"
          part="tool-call"
          data-status=${call.status}
          ?open=${call.status === 'waiting_permission' || this.expandedTools.has(call.id)}
          @toggle=${(event: Event) => this.onToolToggle(call.id, event)}
        >
          <summary>
            <span class="tool-dot"></span>
            <span>${call.title}</span>
          </summary>
          ${call.detail
            ? html`<pre class="tool-detail">${call.detail}</pre>`
            : null}
        </details>`;
      }
      return this.renderWidgets(part.widgets);
    })}`;
  }

  private renderWidgets(widgets: WidgetDefinition[]) {
    const disabled = this.widgetsExpired || this.widgetSubmitted;
    return html`<div class="part widgets" part="widget-panel">
      ${widgets.map((widget) => html`<div class="widget">
        <span class="widget-label">${widget.label}</span>
        ${this.renderWidgetInput(widget, disabled)}
      </div>`)}
      <button
        type="button"
        ?disabled=${disabled || !this.hasWidgetValue(widgets)}
        @click=${() => this.submitWidgets(widgets)}
      >
        ${this.widgetSubmitted ? 'Submitted' : this.widgetsExpired ? 'Expired' : 'Submit'}
      </button>
    </div>`;
  }

  private renderWidgetInput(widget: WidgetDefinition, disabled: boolean) {
    const value = this.widgetValues[widget.raw] ?? '';
    if (widget.kind === 'select:single') {
      return html`<select
        ?disabled=${disabled}
        .value=${String(value)}
        @change=${(event: Event) => this.setWidgetValue(widget, (event.target as HTMLSelectElement).value)}
      >
        <option value="">Select</option>
        ${widget.options.map((option) => html`<option value=${option}>${option}</option>`)}
      </select>`;
    }
    if (widget.kind === 'select:multi') {
      const selected = Array.isArray(value) ? value : [];
      return html`<div class="choice-row">${widget.options.map((option) => html`
        <label>
          <input
            type="checkbox"
            value=${option}
            ?checked=${selected.includes(option)}
            ?disabled=${disabled}
            @change=${(event: Event) => this.toggleMulti(widget, option, event)}
          >
          ${option}
        </label>
      `)}</div>`;
    }
    if (widget.kind === 'confirm') {
      return html`<div class="choice-row">
        <button type="button" ?disabled=${disabled} @click=${() => this.setWidgetValue(widget, 'yes')}>Confirm</button>
        <button type="button" ?disabled=${disabled} @click=${() => this.setWidgetValue(widget, 'no')}>Cancel</button>
      </div>`;
    }
    if (widget.kind === 'input:path') {
      return html`<div class="path-row">
        <input
          type="text"
          .value=${String(value)}
          ?disabled=${disabled}
          @input=${(event: Event) =>
            this.setWidgetValue(widget, (event.target as HTMLInputElement).value)}
        >
        ${this.capabilities.pickPath
          ? html`<button
              type="button"
              ?disabled=${disabled}
              @click=${() => void this.pickPath(widget)}
            >Browse</button>`
          : null}
      </div>`;
    }
    const type = widget.kind === 'input:date'
      ? 'date'
      : widget.kind === 'input:time'
        ? 'time'
        : 'text';
    return html`<input
      type=${type}
      .value=${String(value)}
      ?disabled=${disabled}
      @input=${(event: Event) => this.setWidgetValue(widget, (event.target as HTMLInputElement).value)}
    >`;
  }

  private setWidgetValue(widget: WidgetDefinition, value: string): void {
    this.widgetValues = { ...this.widgetValues, [widget.raw]: value };
  }

  private toggleMulti(widget: WidgetDefinition, option: string, event: Event): void {
    const current = Array.isArray(this.widgetValues[widget.raw])
      ? this.widgetValues[widget.raw] as string[]
      : [];
    const checked = (event.target as HTMLInputElement).checked;
    this.widgetValues = {
      ...this.widgetValues,
      [widget.raw]: checked
        ? [...new Set([...current, option])]
        : current.filter((item) => item !== option),
    };
  }

  private hasWidgetValue(widgets: WidgetDefinition[]): boolean {
    return widgets.some((widget) => {
      const value = this.widgetValues[widget.raw];
      return Array.isArray(value) ? value.length > 0 : Boolean(value);
    });
  }

  private submitWidgets(widgets: WidgetDefinition[]): void {
    if (this.widgetSubmitted || this.widgetsExpired) return;
    const content = widgets
      .map((widget) => [widget.label, this.widgetValues[widget.raw]] as const)
      .filter(([, value]) => Array.isArray(value) ? value.length > 0 : Boolean(value))
      .map(([label, value]) => `${label}: ${Array.isArray(value) ? value.join(', ') : value}`)
      .join('\n');
    if (!content) return;
    this.widgetSubmitted = true;
    this.dispatchEvent(new CustomEvent('agent-widget-submit', {
      bubbles: true,
      composed: true,
      detail: { content },
    }));
  }

  private onToolToggle(id: string, event: Event): void {
    const next = new Set(this.expandedTools);
    if ((event.currentTarget as HTMLDetailsElement).open) next.add(id);
    else next.delete(id);
    this.expandedTools = next;
  }

  private async pickPath(widget: WidgetDefinition): Promise<void> {
    const result = await this.capabilities.pickPath?.({
      mode: 'file',
      label: widget.label,
      accept: widget.accept
        ?.split(',')
        .map((value) => value.trim())
        .filter(Boolean),
      multiple: false,
    });
    const path = result?.paths[0];
    if (path) this.setWidgetValue(widget, path);
  }

  private onMarkdownClick(event: MouseEvent): void {
    const target = event.composedPath()[0];
    if (!(target instanceof HTMLElement)) return;
    const link = target.closest('a');
    if (link instanceof HTMLAnchorElement) {
      const url = link.href;
      if (!isSafeLink(url)) {
        event.preventDefault();
        return;
      }
      if (this.capabilities.openLink) {
        event.preventDefault();
        void this.capabilities.openLink({ url, source: 'markdown' });
      }
      return;
    }
    const image = target.closest('img');
    if (image instanceof HTMLImageElement && this.capabilities.openImage) {
      event.preventDefault();
      void this.capabilities.openImage({
        source: image.getAttribute('src') ?? image.src,
        alt: image.alt,
        baseDir: this.contentBaseDir,
        metadata: this.contentMetadata,
      });
    }
  }

  private startReveal(): void {
    if (this.revealTimer) return;
    this.revealTimer = globalThis.setInterval(() => {
      const remaining = this.content.length - this.visibleLength;
      if (remaining <= 0) {
        this.stopReveal();
        this.emitRevealComplete();
        return;
      }
      const step = remaining > 200 ? 16 : remaining > 80 ? 8 : remaining > 24 ? 4 : 2;
      this.visibleLength = nextRevealLength(this.content, this.visibleLength, step);
      this.dispatchEvent(new CustomEvent('agent-content-reveal', {
        bubbles: true,
        composed: true,
      }));
      if (this.visibleLength >= this.content.length) {
        this.stopReveal();
        this.emitRevealComplete();
      }
    }, 80);
  }

  private stopReveal(): void {
    if (!this.revealTimer) return;
    globalThis.clearInterval(this.revealTimer);
    this.revealTimer = null;
  }

  private emitRevealComplete(): void {
    if (this.completedContent === this.content) return;
    this.completedContent = this.content;
    this.dispatchEvent(new CustomEvent('agent-content-reveal-complete', {
      bubbles: true,
      composed: true,
    }));
  }
}

if (!customElements.get('agent-conversation-rich-content')) {
  customElements.define(
    'agent-conversation-rich-content',
    ConversationRichContentElement,
  );
}

function sourceLength(
  part: ReturnType<typeof parseConversationContent>[number],
): number {
  if (part.kind === 'mermaid') return part.source.length;
  if (part.kind === 'tool') return part.callId.length;
  if (part.kind === 'widgets') {
    return part.widgets.reduce((total, widget) => total + widget.raw.length, 0);
  }
  return part.content.length;
}

function revealBaseline(content: string): number {
  const parts = parseConversationContent(content);
  let lastMarkdownIndex = -1;
  for (let index = parts.length - 1; index >= 0; index -= 1) {
    if (parts[index].kind === 'markdown') {
      lastMarkdownIndex = index;
      break;
    }
  }
  if (lastMarkdownIndex < 0) return content.length;
  return parts
    .slice(0, lastMarkdownIndex)
    .reduce((total, part) => total + sourceLength(part), 0);
}

function nextRevealLength(content: string, current: number, step: number): number {
  let next = Math.min(content.length, current + step);
  const atomicInlinePattern = /!?\[[^\]]*\]\([^)]+\)/g;
  let match: RegExpExecArray | null;
  while ((match = atomicInlinePattern.exec(content)) !== null) {
    const end = match.index + match[0].length;
    if (next > match.index && next < end) {
      next = end;
      break;
    }
  }
  return next;
}

function isSafeLink(url: string): boolean {
  try {
    return ['http:', 'https:', 'mailto:'].includes(new URL(url).protocol);
  } catch {
    return false;
  }
}
