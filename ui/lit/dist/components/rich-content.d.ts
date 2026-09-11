import { LitElement } from 'lit';
import type { ConversationHostCapabilities } from '../host/types.js';
import type { ToolCallView } from '../protocol/types.js';
import './mermaid-diagram.js';
export declare class ConversationRichContentElement extends LitElement {
    static properties: {
        content: {
            type: StringConstructor;
        };
        toolCalls: {
            attribute: boolean;
        };
        colorScheme: {
            type: StringConstructor;
            attribute: string;
        };
        capabilities: {
            attribute: boolean;
        };
        contentBaseDir: {
            attribute: boolean;
        };
        contentMetadata: {
            attribute: boolean;
        };
        hideToolCalls: {
            type: BooleanConstructor;
            attribute: string;
        };
        locale: {
            type: StringConstructor;
        };
        reveal: {
            type: BooleanConstructor;
        };
        widgetsExpired: {
            type: BooleanConstructor;
            attribute: string;
        };
        visibleLength: {
            state: boolean;
        };
        expandedTools: {
            state: boolean;
        };
        widgetValues: {
            state: boolean;
        };
        widgetSubmitted: {
            state: boolean;
        };
    };
    static styles: import("lit").CSSResult;
    content: string;
    toolCalls: ToolCallView[];
    colorScheme: string;
    capabilities: ConversationHostCapabilities;
    contentBaseDir: string | undefined;
    contentMetadata: Record<string, unknown> | undefined;
    hideToolCalls: boolean;
    locale: string;
    reveal: boolean;
    widgetsExpired: boolean;
    private visibleLength;
    private expandedTools;
    private widgetValues;
    private widgetSubmitted;
    private revealTimer;
    private completedContent;
    constructor();
    disconnectedCallback(): void;
    protected willUpdate(changed: Map<PropertyKey, unknown>): void;
    protected updated(changed: Map<PropertyKey, unknown>): void;
    protected render(): import("lit-html").TemplateResult<1>;
    private renderWidgets;
    private renderWidgetInput;
    private setWidgetValue;
    private toggleMulti;
    private hasWidgetValue;
    private submitWidgets;
    private onToolToggle;
    private pickPath;
    private onMarkdownClick;
    private startReveal;
    private stopReveal;
    private emitRevealComplete;
}
//# sourceMappingURL=rich-content.d.ts.map