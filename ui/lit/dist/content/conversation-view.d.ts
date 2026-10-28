import type { ConversationPresentationItem } from '../host/types.js';
import { type ConversationState, type LedgerRecord } from '../protocol/index.js';
export type DisplayItem = {
    kind: 'record';
    key: string;
    record: LedgerRecord;
} | {
    kind: 'presentation';
    key: string;
    item: ConversationPresentationItem;
};
export declare function createDisplayItems(state: ConversationState, presentationItems: readonly ConversationPresentationItem[], completedRevealKeys: Set<string>): DisplayItem[];
export declare function latestWidgetRecordKey(items: DisplayItem[]): string | null;
export declare function latestAssistantKey(items: DisplayItem[]): string | null;
//# sourceMappingURL=conversation-view.d.ts.map