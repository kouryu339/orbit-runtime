import { LitElement, css, html } from 'lit';
import { isSafeImageSource } from '../content/references.js';
export class ImageReferenceElement extends LitElement {
    static properties = {
        imageId: { type: String },
        conversationId: { type: String },
        capabilities: { attribute: false },
        label: { type: String },
        source: { state: true },
        alt: { state: true },
        failed: { state: true },
    };
    controller;
    constructor() {
        super();
        this.imageId = '';
        this.capabilities = {};
        this.label = 'Image';
        this.source = '';
        this.alt = '';
        this.failed = false;
    }
    static styles = css `
    :host {
      display: inline-block;
      vertical-align: middle;
      margin: 3px 4px;
    }
    button {
      display: inline-grid;
      place-items: center;
      width: 64px;
      height: 48px;
      padding: 0;
      overflow: hidden;
      border: 1px solid var(--reference-color, var(--conversation-border));
      border-radius: 7px;
      background: var(--conversation-code-background, #edf1ef);
      color: inherit;
      cursor: pointer;
      vertical-align: middle;
    }
    button:disabled {
      cursor: default;
    }
    button:focus-visible {
      outline: 2px solid var(--conversation-accent, #167d71);
      outline-offset: 3px;
    }
    img {
      display: block;
      width: 100%;
      height: 100%;
      object-fit: cover;
    }
    .fallback {
      display: inline;
      padding: 2px 5px;
      border-radius: 5px;
      background: var(--reference-color, #27624f);
      color: #fff;
      white-space: nowrap;
    }
  `;
    willUpdate(changed) {
        if (changed.has('imageId') ||
            changed.has('conversationId') ||
            changed.has('capabilities'))
            void this.load();
    }
    disconnectedCallback() {
        this.controller?.abort();
        super.disconnectedCallback();
    }
    connectedCallback() {
        super.connectedCallback();
        if (this.hasUpdated)
            void this.load();
    }
    async load() {
        this.controller?.abort();
        const controller = new AbortController();
        this.controller = controller;
        this.source = '';
        this.alt = '';
        this.failed = false;
        try {
            const result = await this.capabilities.resolveImage?.({
                imageId: this.imageId,
                conversationId: this.conversationId,
                signal: controller.signal,
            });
            if (controller.signal.aborted)
                return;
            if (result && isSafeImageSource(result.source)) {
                this.source = result.source;
                this.alt = result.alt ?? '';
            }
        }
        catch (error) {
            if (!controller.signal.aborted)
                this.report(error);
        }
    }
    report(error) {
        this.dispatchEvent(new CustomEvent('agent-conversation-error', {
            bubbles: true,
            composed: true,
            detail: {
                code: 'image-preview-failed',
                message: error instanceof Error ? error.message : String(error),
                recoverable: true,
            },
        }));
    }
    async open() {
        try {
            await this.capabilities.openImage?.({
                source: this.source,
                alt: this.alt || this.label,
                metadata: { imageId: this.imageId },
            });
        }
        catch (error) {
            this.report(error);
        }
    }
    render() {
        return this.source && !this.failed
            ? html `<button
          type="button"
          aria-label=${this.alt || this.label}
          title=${this.alt || this.label}
          ?disabled=${!this.capabilities.openImage}
          @click=${() => void this.open()}
        >
          <img
            src=${this.source}
            alt=${this.alt || this.label}
            @error=${() => {
                this.failed = true;
            }}
          />
        </button>`
            : html `<span class="fallback"><slot>${this.label}</slot></span>`;
    }
}
if (!customElements.get('agent-runtime-image-reference'))
    customElements.define('agent-runtime-image-reference', ImageReferenceElement);
//# sourceMappingURL=image-reference.js.map