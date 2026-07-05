import { LitElement, nothing } from 'lit';
export declare class MermaidDiagramElement extends LitElement {
    static properties: {
        source: {
            type: StringConstructor;
        };
        colorScheme: {
            type: StringConstructor;
            attribute: string;
        };
        renderedSvg: {
            state: boolean;
        };
        error: {
            state: boolean;
        };
    };
    static styles: import("lit").CSSResult;
    source: string;
    colorScheme: string;
    private renderedSvg;
    private error;
    private renderVersion;
    constructor();
    protected updated(changed: Map<PropertyKey, unknown>): void;
    protected render(): import("lit-html").TemplateResult<1> | typeof nothing;
    private renderDiagram;
}
//# sourceMappingURL=mermaid-diagram.d.ts.map