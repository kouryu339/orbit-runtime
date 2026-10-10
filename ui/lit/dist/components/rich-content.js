import { LitElement, css, html } from 'lit';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';
import { parseConversationContent, toolCallForId, } from '../content/parser.js';
import { renderSafeMarkdown } from '../content/markdown.js';
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
        otherEnabled: { state: true },
        otherValues: { state: true },
    };
    static styles = css `
    :host {
      display: block;
      min-width: 0;
      max-width: 100%;
      overflow-x: hidden;
      color: var(--conversation-text);
    }

    .markdown {
      min-width: 0;
      max-width: 100%;
      overflow-x: hidden;
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
      max-width: 100%;
      overflow-x: hidden;
      padding: 13px;
      border: 1px solid var(--conversation-border);
      border-radius: 10px;
      background: var(--conversation-code-background);
      font: 12px/1.55 var(--conversation-font-mono);
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      word-break: break-word;
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
      max-width: 100%;
      overflow-x: hidden;
      margin: 14px 0;
      text-align: center;
    }

    .part + .part {
      margin-top: 12px;
    }

    .tool {
      min-width: 0;
      max-width: 100%;
      overflow-x: hidden;
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
      overflow-wrap: anywhere;
      word-break: break-word;
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
    .tool[data-status="uncertain"] .tool-dot {
      background: var(--conversation-accent);
      box-shadow: 0 0 0 3px color-mix(in srgb, var(--conversation-accent) 18%, transparent);
    }
    .tool[data-status="waiting_permission"] {
      border-color: color-mix(in srgb, var(--conversation-accent) 45%, var(--conversation-border));
    }
    .tool[data-status="waiting_permission"] .tool-dot {
      background: var(--conversation-accent);
      box-shadow: 0 0 0 3px color-mix(in srgb, var(--conversation-accent) 18%, transparent);
    }
    .tool-detail {
      max-width: 100%;
      overflow-x: hidden;
      margin: 0;
      padding: 0 11px 11px 27px;
      color: var(--conversation-text-muted);
      font: 12px/1.55 var(--conversation-font-mono);
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      word-break: break-word;
    }
    .widgets {
      display: grid;
      min-width: 0;
      max-width: 100%;
      gap: 18px;
      padding: 4px;
      box-sizing: border-box;
      grid-template-columns: minmax(0, 1fr);
    }
    .widget { display: grid; grid-template-columns: minmax(0, 1fr); gap: 10px; min-width: 0; }
    .widget > input, .path-row > input { width: 100%; max-width: 100%; }
    .widget-label {
      color: var(--conversation-text-strong);
      font: 600 13px/1.5 var(--conversation-font-body);
    }
    input {
      box-sizing: border-box;
      min-width: 0;
      height: 42px;
      padding: 0 12px;
      border: 1px solid var(--conversation-border);
      border-radius: 10px;
      background: transparent;
      color: var(--conversation-text);
      font: 14px var(--conversation-font-body);
      caret-color: var(--conversation-text-strong);
    }
    input::placeholder { color: var(--conversation-text-muted); opacity: 1; }
    button {
      min-height: 36px;
      padding: 7px 16px;
      border: 1px solid var(--conversation-border);
      border-radius: 9px;
      background: var(--conversation-surface);
      color: var(--conversation-text-strong);
      font: 600 13px/1.4 var(--conversation-font-body);
      cursor: pointer;
    }
    .widgets > button {
      justify-self: end;
      min-width: 72px;
      background: var(--conversation-text-strong);
      color: var(--conversation-surface);
      border-color: transparent;
      margin-top: 2px;
    }
    button:disabled, input:disabled { opacity: 0.5; cursor: not-allowed; }
    .choice-list { display: grid; gap: 3px; min-width: 0; }
    .choice-option {
      position: relative;
      box-sizing: border-box;
      min-width: 0;
      display: flex;
      align-items: center;
      gap: 12px;
      min-height: 42px;
      padding: 9px 12px;
      border: 1px solid transparent;
      border-radius: 10px;
      cursor: pointer;
      font: 14px/1.55 var(--conversation-font-body);
    }
    .choice-option span { min-width: 0; overflow-wrap: anywhere; }
    .choice-option:hover:not(:has(input:disabled)) {
      background: color-mix(in srgb, var(--conversation-text) 5%, transparent);
    }
    .choice-option:has(input:checked) {
      background: color-mix(in srgb, var(--conversation-accent) 18%, var(--conversation-surface));
      border-color: var(--conversation-accent);
      color: var(--conversation-text-strong);
    }
    .choice-option:has(input:disabled) { cursor: default; }
    .choice-option input[type="radio"], .choice-option input[type="checkbox"] {
      position: absolute;
      width: 1px;
      height: 1px;
      padding: 0;
      margin: 0;
      border: 0;
      overflow: hidden;
      clip-path: inset(50%);
      white-space: nowrap;
    }
    .choice-option:has(input:focus-visible) {
      outline: 2px solid var(--conversation-accent);
      outline-offset: 2px;
    }
    .other-option {
      margin-top: 7px;
      border-color: var(--conversation-border);
      cursor: text;
      padding-block: 7px;
    }
    .other-option:focus-within { border-color: var(--conversation-text-muted); }
    .other-option .other-input {
      flex: 1;
      width: 0;
      min-width: 0;
      height: 28px;
      padding: 0;
      border: 0;
      background: transparent;
      border-radius: 0;
    }
    input:focus-visible, button:focus-visible {
      outline: 2px solid var(--conversation-accent);
      outline-offset: 3px;
    }
    .other-option .other-input:focus-visible { outline: none; }
    .other-option:has(.other-input:focus-visible) {
      outline: 2px solid var(--conversation-accent);
      outline-offset: 2px;
    }
    .choice-row { display: flex; justify-content: center; flex-wrap: wrap; gap: 24px; }
    .choice-row button { min-width: 96px; }
    .choice-row button[aria-pressed="true"] {
      background: var(--conversation-text-strong);
      color: var(--conversation-surface);
      border-color: transparent;
    }
    .path-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; }
    .path-row button { min-height: 42px; }
    .widgets[data-inactive="true"] {
      color: var(--conversation-text-muted);
    }
    .widgets[data-inactive="true"] .widget-label,
    .widgets[data-inactive="true"] input,
    .widgets[data-inactive="true"] button {
      color: var(--conversation-text-muted);
    }
    .widgets[data-inactive="true"] input:disabled,
    .widgets[data-inactive="true"] button:disabled {
      opacity: 1;
      cursor: default;
    }
    .widgets[data-inactive="true"] .choice-option {
      cursor: default;
    }
    .widgets[data-inactive="true"] .choice-option:has(input:checked),
    .widgets[data-inactive="true"] .choice-row button[aria-pressed="true"] {
      color: var(--conversation-text-muted);
      background: color-mix(in srgb, var(--conversation-text-muted) 10%, transparent);
      border-color: var(--conversation-border);
    }
    .widget-status {
      padding: 8px 4px;
      color: var(--conversation-text-muted);
      font: 12px/1.5 var(--conversation-font-body);
    }
    @media (max-width: 480px) {
      input { font-size: 16px; }
      .choice-option { padding-inline: 9px; }
    }
    @media (forced-colors: active) {
      .choice-option:has(input:checked) { border-color: Highlight; outline: 2px solid Highlight; outline-offset: -3px; }
    }
  `;
    revealTimer = null;
    completedContent = '';
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
        this.otherEnabled = new Set();
        this.otherValues = {};
    }
    disconnectedCallback() {
        super.disconnectedCallback();
        this.stopReveal();
    }
    willUpdate(changed) {
        if (!changed.has('content') && !changed.has('reveal'))
            return;
        if (!this.reveal)
            return;
        const previous = String(changed.get('content') ?? '');
        const baseline = revealBaseline(this.content);
        if (!previous || !this.content.startsWith(previous)) {
            this.visibleLength = baseline;
        }
        else {
            this.visibleLength = Math.max(this.visibleLength, baseline);
        }
        if (this.visibleLength > this.content.length) {
            this.visibleLength = this.content.length;
        }
    }
    updated(changed) {
        if (!changed.has('content') && !changed.has('reveal'))
            return;
        if (!this.reveal) {
            this.stopReveal();
            queueMicrotask(() => this.emitRevealComplete());
            return;
        }
        if (this.visibleLength < this.content.length)
            this.startReveal();
        else
            this.emitRevealComplete();
    }
    render() {
        const parts = parseConversationContent(this.content);
        const widgets = parts.flatMap((part) => part.kind === 'widgets' ? part.widgets : []);
        let lastMarkdownIndex = -1;
        for (let index = parts.length - 1; index >= 0; index -= 1) {
            if (parts[index].kind === 'markdown') {
                lastMarkdownIndex = index;
                break;
            }
        }
        let consumed = 0;
        return html `${parts.map((part, partIndex) => {
            if (part.kind === 'markdown') {
                let content = part.content;
                if (this.reveal && partIndex === lastMarkdownIndex) {
                    const available = Math.max(0, this.visibleLength - consumed);
                    content = part.content.slice(0, available);
                }
                consumed += part.content.length;
                return html `<div class="part markdown" part="markdown">
          <div @click=${this.onMarkdownClick}>
            ${unsafeHTML(renderSafeMarkdown(content))}
          </div>
        </div>`;
            }
            consumed += sourceLength(part);
            if (part.kind === 'mermaid') {
                return html `<agent-mermaid-diagram
          class="part"
          .source=${part.source}
          .colorScheme=${this.colorScheme}
        ></agent-mermaid-diagram>`;
            }
            if (part.kind === 'tool') {
                if (this.hideToolCalls)
                    return null;
                const call = toolCallForId(this.toolCalls, part.callId);
                return html `<details
          class="part tool"
          part="tool-call"
          data-status=${call.status}
          ?open=${call.status === 'waiting_permission' || this.expandedTools.has(call.id)}
          @toggle=${(event) => this.onToolToggle(call.id, event)}
        >
          <summary>
            <span class="tool-dot"></span>
            <span>${call.title}</span>
          </summary>
          ${call.detail
                    ? html `<pre class="tool-detail">${call.detail}</pre>`
                    : null}
        </details>`;
            }
            return this.renderWidgets(part.widgets);
        })}
    ${widgets.length ? this.renderWidgetSubmit(widgets) : null}`;
    }
    renderWidgets(widgets) {
        const disabled = this.widgetsExpired || this.widgetSubmitted;
        return html `<div class="part widgets" part="widget-panel" data-inactive=${disabled ? 'true' : 'false'}>
      ${widgets.map((widget) => html `<div class="widget">
        <span class="widget-label">${widget.label}</span>
        ${this.renderWidgetInput(widget, disabled)}
      </div>`)}
    </div>`;
    }
    renderWidgetSubmit(widgets) {
        if (this.widgetSubmitted || this.widgetsExpired) {
            return html `<div class="part widget-status" role="status">
        ${this.widgetSubmitted
                ? this.label('已提交', 'Submitted')
                : this.label('已过期 · 请在最新消息中回复', 'Expired · Reply in the latest message')}
      </div>`;
        }
        return html `<div class="part widgets widget-actions">
      <button
        type="button"
        ?disabled=${!this.hasWidgetValue(widgets)}
        @click=${() => this.submitWidgets(widgets)}
      >
        ${this.label('提交', 'Submit')}
      </button>
    </div>`;
    }
    renderWidgetInput(widget, disabled) {
        const value = this.widgetValues[widget.raw] ?? '';
        if (widget.kind === 'select:single' || widget.kind === 'select:multi') {
            const multiple = widget.kind === 'select:multi';
            const other = this.otherEnabled.has(widget.raw);
            const selected = Array.isArray(value) ? value : [value];
            const options = [...new Set(widget.options)].filter((option) => !/^(其他|其它|other)$/i.test(option.trim()));
            return html `<div class="choice-list" role="group" aria-label=${widget.label}>
        ${options.map((option) => html `<label class="choice-option">
          <input type=${multiple ? 'checkbox' : 'radio'} name=${encodeURIComponent(widget.raw)}
            ?checked=${selected.includes(option) && (multiple || !other)} ?disabled=${disabled}
            @change=${(event) => {
                if (multiple)
                    this.toggleMulti(widget, option, event);
                else {
                    this.setOther(widget, false);
                    this.setWidgetValue(widget, option);
                }
            }}><span>${option}</span>
        </label>`)}

        <div class="choice-option other-option">
          <input class="other-toggle" type=${multiple ? 'checkbox' : 'radio'} name=${encodeURIComponent(widget.raw)}
            aria-label=${widget.label + ': ' + this.label('其他', 'Other')}
            ?checked=${other} ?disabled=${disabled}
            @change=${(event) => this.setOther(widget, event.target.checked)}>
          <input class="other-input" type="text"
            aria-label=${widget.label + ': ' + this.label('其他内容', 'Other answer')}
            placeholder=${this.label('其他，请输入…', 'Other, please specify…')}
            .value=${this.otherValues[widget.raw] ?? ''} ?disabled=${disabled}
            @focus=${() => { if (!disabled)
                this.setOther(widget, true); }}
            @input=${(event) => {
                this.setOther(widget, true);
                this.otherValues = { ...this.otherValues, [widget.raw]: event.target.value };
            }}>
        </div>
      </div>`;
        }
        if (widget.kind === 'confirm') {
            return html `<div class="choice-row">
        <button type="button" ?disabled=${disabled} aria-pressed=${value === 'yes'} @click=${() => this.setWidgetValue(widget, 'yes')}>${this.label('确认', 'Confirm')}</button>
        <button type="button" ?disabled=${disabled} aria-pressed=${value === 'no'} @click=${() => this.setWidgetValue(widget, 'no')}>${this.label('取消', 'Cancel')}</button>
      </div>`;
        }
        if (widget.kind === 'input:path') {
            return html `<div class="path-row">
        <input
          type="text"
          aria-label=${widget.label}
          .value=${String(value)}
          ?disabled=${disabled}
          @input=${(event) => this.setWidgetValue(widget, event.target.value)}
        >
        ${this.capabilities.pickPath
                ? html `<button
              type="button"
              ?disabled=${disabled}
              @click=${() => void this.pickPath(widget)}
            >${this.label('浏览…', 'Browse…')}</button>`
                : null}
      </div>`;
        }
        return html `<input
      type="text"
      aria-label=${widget.label}
      .value=${String(value)}
      ?disabled=${disabled}
      @input=${(event) => this.setWidgetValue(widget, event.target.value)}
    >`;
    }
    label(zh, en) {
        return this.locale.toLowerCase().startsWith('zh') ? zh : en;
    }
    setOther(widget, enabled) {
        const next = new Set(this.otherEnabled);
        if (enabled)
            next.add(widget.raw);
        else
            next.delete(widget.raw);
        this.otherEnabled = next;
        if (enabled && widget.kind === 'select:single')
            this.setWidgetValue(widget, '');
    }
    answer(widget) {
        const value = this.widgetValues[widget.raw] ?? '';
        if (!this.otherEnabled.has(widget.raw))
            return typeof value === 'string' ? value.trim() : value;
        const other = (this.otherValues[widget.raw] ?? '').trim();
        return widget.kind === 'select:multi'
            ? [...new Set([...(Array.isArray(value) ? value : []), ...(other ? [other] : [])])]
            : other;
    }
    setWidgetValue(widget, value) {
        this.widgetValues = { ...this.widgetValues, [widget.raw]: value };
    }
    toggleMulti(widget, option, event) {
        const current = Array.isArray(this.widgetValues[widget.raw])
            ? this.widgetValues[widget.raw]
            : [];
        const checked = event.target.checked;
        this.widgetValues = {
            ...this.widgetValues,
            [widget.raw]: checked
                ? [...new Set([...current, option])]
                : current.filter((item) => item !== option),
        };
    }
    hasWidgetValue(widgets) {
        if (widgets.some((widget) => this.otherEnabled.has(widget.raw)
            && !(this.otherValues[widget.raw] ?? '').trim()))
            return false;
        return widgets.some((widget) => {
            const value = this.answer(widget);
            return Array.isArray(value) ? value.length > 0 : Boolean(value);
        });
    }
    submitWidgets(widgets) {
        if (this.widgetSubmitted || this.widgetsExpired || !this.hasWidgetValue(widgets))
            return;
        const content = widgets
            .map((widget) => [widget.label, this.answer(widget)])
            .filter(([, value]) => Array.isArray(value) ? value.length > 0 : Boolean(value))
            .map(([label, value]) => `${label}: ${Array.isArray(value) ? value.join(', ') : value}`)
            .join('\n');
        if (!content)
            return;
        this.widgetSubmitted = true;
        this.dispatchEvent(new CustomEvent('agent-widget-submit', {
            bubbles: true,
            composed: true,
            detail: { content },
        }));
    }
    onToolToggle(id, event) {
        const next = new Set(this.expandedTools);
        if (event.currentTarget.open)
            next.add(id);
        else
            next.delete(id);
        this.expandedTools = next;
    }
    async pickPath(widget) {
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
        if (path)
            this.setWidgetValue(widget, path);
    }
    onMarkdownClick(event) {
        const target = event.composedPath()[0];
        if (!(target instanceof HTMLElement))
            return;
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
    startReveal() {
        if (this.revealTimer)
            return;
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
    stopReveal() {
        if (!this.revealTimer)
            return;
        globalThis.clearInterval(this.revealTimer);
        this.revealTimer = null;
    }
    emitRevealComplete() {
        if (this.completedContent === this.content)
            return;
        this.completedContent = this.content;
        this.dispatchEvent(new CustomEvent('agent-content-reveal-complete', {
            bubbles: true,
            composed: true,
        }));
    }
}
if (!customElements.get('agent-conversation-rich-content')) {
    customElements.define('agent-conversation-rich-content', ConversationRichContentElement);
}
function sourceLength(part) {
    if (part.kind === 'mermaid')
        return part.source.length;
    if (part.kind === 'tool')
        return part.callId.length;
    if (part.kind === 'widgets') {
        return part.widgets.reduce((total, widget) => total + widget.raw.length, 0);
    }
    return part.content.length;
}
function revealBaseline(content) {
    const parts = parseConversationContent(content);
    let lastMarkdownIndex = -1;
    for (let index = parts.length - 1; index >= 0; index -= 1) {
        if (parts[index].kind === 'markdown') {
            lastMarkdownIndex = index;
            break;
        }
    }
    if (lastMarkdownIndex < 0)
        return content.length;
    return parts
        .slice(0, lastMarkdownIndex)
        .reduce((total, part) => total + sourceLength(part), 0);
}
function nextRevealLength(content, current, step) {
    let next = Math.min(content.length, current + step);
    const atomicInlinePattern = /!?\[[^\]]*\]\([^)]+\)/g;
    let match;
    while ((match = atomicInlinePattern.exec(content)) !== null) {
        const end = match.index + match[0].length;
        if (next > match.index && next < end) {
            next = end;
            break;
        }
    }
    return next;
}
function isSafeLink(url) {
    try {
        return ['http:', 'https:', 'mailto:'].includes(new URL(url).protocol);
    }
    catch {
        return false;
    }
}
//# sourceMappingURL=rich-content.js.map