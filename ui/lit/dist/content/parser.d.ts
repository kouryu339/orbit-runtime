import type { ToolCallView } from '../protocol/types.js';
export type WidgetKind = 'input:path' | 'input:text' | 'select:single' | 'select:multi' | 'confirm';
export type WidgetDefinition = {
    kind: WidgetKind;
    label: string;
    options: string[];
    accept?: string;
    raw: string;
};
export type ConversationContentPart = {
    kind: 'markdown';
    content: string;
} | {
    kind: 'mermaid';
    source: string;
} | {
    kind: 'tool';
    callId: string;
} | {
    kind: 'widgets';
    widgets: WidgetDefinition[];
};
export declare function parseConversationContent(content: string): ConversationContentPart[];
export declare function parseWidget(line: string): WidgetDefinition | null;
export declare function toolCallForId(calls: ToolCallView[], callId: string): ToolCallView;
export declare function toolStatusLabel(status: ToolCallView['status'], locale: string): string;
/** No raw-result, raw-error or command fallback, including for legacy callers. */
export declare function toolDisplaySummary(call: ToolCallView, locale: string): string;
//# sourceMappingURL=parser.d.ts.map