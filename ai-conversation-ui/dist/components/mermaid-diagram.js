import DOMPurify from 'dompurify';
import { LitElement, css, html, nothing } from 'lit';
import { unsafeSVG } from 'lit/directives/unsafe-svg.js';
let diagramSequence = 0;
export class MermaidDiagramElement extends LitElement {
    static properties = {
        source: { type: String },
        colorScheme: { type: String, attribute: 'color-scheme' },
        renderedSvg: { state: true },
        error: { state: true },
    };
    static styles = css `
    :host {
      display: block;
    }

    .diagram {
      overflow: auto;
      padding: 14px;
      border: 1px solid var(--conversation-border);
      border-radius: 12px;
      background: var(--conversation-surface-raised);
    }

    .diagram svg {
      display: block;
      min-width: 320px;
      max-width: 100%;
      height: auto;
      margin: 0 auto;
    }

    pre {
      overflow: auto;
      margin: 0;
      padding: 12px;
      border: 1px solid var(--conversation-tool-error);
      border-radius: 10px;
      background: var(--conversation-code-background);
      color: var(--conversation-text);
      font: 12px/1.55 var(--conversation-font-mono);
      white-space: pre;
    }
  `;
    renderVersion = 0;
    constructor() {
        super();
        this.source = '';
        this.colorScheme = 'light';
        this.renderedSvg = '';
        this.error = '';
    }
    updated(changed) {
        if (changed.has('source') || changed.has('colorScheme'))
            void this.renderDiagram();
    }
    render() {
        if (this.error) {
            return html `<pre part="mermaid-fallback"><code>${this.source}</code></pre>`;
        }
        return this.renderedSvg
            ? html `<div class="diagram" part="mermaid-diagram">
          ${unsafeSVG(this.renderedSvg)}
        </div>`
            : nothing;
    }
    async renderDiagram() {
        const source = this.source.trim();
        const version = ++this.renderVersion;
        if (!source) {
            this.renderedSvg = '';
            this.error = '';
            return;
        }
        try {
            const { default: mermaid } = await import('mermaid');
            mermaid.initialize({
                startOnLoad: false,
                securityLevel: 'strict',
                theme: this.colorScheme === 'dark' ? 'dark' : 'neutral',
                flowchart: { htmlLabels: false },
            });
            const id = `agent-conversation-diagram-${++diagramSequence}`;
            const result = await mermaid.render(id, source);
            if (version !== this.renderVersion)
                return;
            this.renderedSvg = DOMPurify.sanitize(result.svg, {
                USE_PROFILES: { svg: true, svgFilters: true },
                FORBID_TAGS: ['script', 'foreignObject', 'iframe', 'object', 'embed'],
                FORBID_ATTR: ['onload', 'onclick', 'onerror'],
            });
            this.error = '';
        }
        catch (error) {
            if (version !== this.renderVersion)
                return;
            this.renderedSvg = '';
            this.error = error instanceof Error ? error.message : String(error);
            this.dispatchEvent(new CustomEvent('agent-conversation-diagnostic', {
                bubbles: true,
                composed: true,
                detail: {
                    code: 'mermaid-render-failed',
                    message: this.error,
                },
            }));
        }
    }
}
if (!customElements.get('agent-mermaid-diagram')) {
    customElements.define('agent-mermaid-diagram', MermaidDiagramElement);
}
//# sourceMappingURL=mermaid-diagram.js.map