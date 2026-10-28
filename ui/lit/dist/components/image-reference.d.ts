import { LitElement } from 'lit';
import type { ConversationHostCapabilities } from '../host/types.js';
export declare class ImageReferenceElement extends LitElement {
    static properties: {
        imageId: {
            type: StringConstructor;
        };
        conversationId: {
            type: StringConstructor;
        };
        capabilities: {
            attribute: boolean;
        };
        label: {
            type: StringConstructor;
        };
        source: {
            state: boolean;
        };
        alt: {
            state: boolean;
        };
        failed: {
            state: boolean;
        };
    };
    imageId: string;
    conversationId: string | undefined;
    capabilities: ConversationHostCapabilities;
    label: string;
    private source;
    private alt;
    private failed;
    private controller;
    constructor();
    static styles: import("lit").CSSResult;
    protected willUpdate(changed: Map<PropertyKey, unknown>): void;
    disconnectedCallback(): void;
    connectedCallback(): void;
    private load;
    private report;
    private open;
    protected render(): import("lit-html").TemplateResult<1>;
}
//# sourceMappingURL=image-reference.d.ts.map