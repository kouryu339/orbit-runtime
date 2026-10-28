import { visiblePresentationItems } from './presentation.js';
import { recordKey, displayText, } from '../protocol/index.js';
function isVisibleGatewayRecord(record) {
    return (record.role === 'gateway_message' &&
        ['llm_error', 'interrupted'].includes(record.metadata?.subtype ?? ''));
}
export function createDisplayItems(state, presentationItems, completedRevealKeys) {
    const records = state.records
        .filter((record) => record.role === 'user' ||
        record.role === 'assistant' ||
        isVisibleGatewayRecord(record))
        .map((record) => ({ kind: 'record', key: recordKey(record), record }));
    const items = [...records];
    const anchorInsertCounts = new Map();
    for (const presentation of visiblePresentationItems(presentationItems, completedRevealKeys)) {
        const presentationKey = `presentation:${presentation.id}`;
        const item = {
            kind: 'presentation',
            key: presentationKey,
            item: presentation,
        };
        if (presentation.anchor.type === 'head') {
            const offset = anchorInsertCounts.get('head') ?? 0;
            items.splice(offset, 0, item);
            anchorInsertCounts.set('head', offset + 1);
        }
        else if (presentation.anchor.type === 'tail') {
            items.push(item);
        }
        else {
            const anchor = presentation.anchor;
            const target = items.findIndex((current) => current.kind === 'record' &&
                recordKey(current.record) === anchor.recordId);
            if (target < 0)
                items.push(item);
            else {
                const anchorKey = `${anchor.type}:${anchor.recordId}`;
                const offset = anchorInsertCounts.get(anchorKey) ?? 0;
                items.splice(anchor.type === 'before-record' ? target : target + 1 + offset, 0, item);
                anchorInsertCounts.set(anchorKey, offset + 1);
            }
        }
    }
    for (const pending of state.pendingUserMessages) {
        items.push({
            kind: 'record',
            key: pending.id,
            record: {
                record_id: pending.id,
                role: 'user',
                content: pending.state === 'failed'
                    ? `${pending.content}\n${pending.error ?? ''}`
                    : pending.content,
                metadata: pending.parts
                    ? {
                        extra: {
                            parts: pending.parts,
                            pending_error: pending.state === 'failed'
                                ? (pending.error ?? 'Message rejected.')
                                : undefined,
                        },
                    }
                    : undefined,
            },
        });
    }
    const stream = state.assistantStream;
    const provisionalToolLines = (stream?.provisional_tool_calls ?? []).map((call) => `[tool:status | call_id="${call.call_id || call.key}"]`);
    if (stream && (stream.content || provisionalToolLines.length)) {
        items.push({
            kind: 'record',
            key: `assistant-stream:${stream.agent_id}:${stream.turn_id}:${stream.attempt}`,
            record: {
                record_id: `assistant-stream:${stream.agent_id}:${stream.turn_id}:${stream.attempt}`,
                role: 'assistant',
                content: [stream.content, ...provisionalToolLines]
                    .filter(Boolean)
                    .join('\n'),
            },
        });
    }
    return items;
}
export function latestWidgetRecordKey(items) {
    for (let index = items.length - 1; index >= 0; index -= 1) {
        const item = items[index];
        if (item.kind !== 'record')
            continue;
        if (item.record.role !== 'assistant' && item.record.role !== 'user')
            continue;
        return item.record.role === 'assistant' &&
            /\[(?:input:|select:|confirm\b)/.test(displayText(item.record))
            ? item.key
            : null;
    }
    return null;
}
export function latestAssistantKey(items) {
    for (let index = items.length - 1; index >= 0; index -= 1) {
        const item = items[index];
        if (item.kind === 'record' && item.record.role === 'assistant') {
            return item.key;
        }
    }
    return null;
}
//# sourceMappingURL=conversation-view.js.map