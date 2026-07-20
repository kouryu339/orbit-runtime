import type { FrontendSnapshotPayload, LedgerRecord, ToolCallView } from './types.js';
export declare const TOOL_PLACEHOLDER_PATTERN: RegExp;
export declare function recordKey(record: LedgerRecord): string;
export declare function recordText(record: LedgerRecord): string;
export declare function displayText(record: LedgerRecord): string;
export declare function snapshotRecords(payload: FrontendSnapshotPayload): LedgerRecord[];
export declare function dedupeRecords(records: LedgerRecord[]): LedgerRecord[];
export declare function toolCallIdsFromRecord(record: LedgerRecord): string[];
export declare function toolCallFromRecord(record: LedgerRecord): ToolCallView | null;
export declare function collectToolCalls(records: LedgerRecord[], previous?: ToolCallView[]): ToolCallView[];
export declare function isRenderableAssistantRecord(record: LedgerRecord): boolean;
//# sourceMappingURL=records.d.ts.map