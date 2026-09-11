import type {
  FrontendSnapshotPayload,
  LedgerRecord,
  ToolCallView,
} from './types.js';

export const TOOL_PLACEHOLDER_PATTERN =
  /\[tool:status\s*\|\s*call_id="([^"]+)"\]/g;

export function recordKey(record: LedgerRecord): string {
  return String(
    record.record_id ??
      record.id ??
      `${record.role ?? 'record'}-${record.created_at ?? ''}-${recordText(record)}`,
  );
}

export function recordText(record: LedgerRecord): string {
  return record.content ?? record.text ?? '';
}

export function displayText(record: LedgerRecord): string {
  return typeof record.metadata?.display_content === 'string'
    ? record.metadata.display_content
    : recordText(record);
}

/**
 * Return assistant content with stable anchors for metadata-only native calls.
 * Native FC declarations are not serialized into the assistant's text, but a
 * rich-content renderer still needs one anchor per call to place its bubble.
 */
export function displayTextWithToolAnchors(record: LedgerRecord): string {
  const content = displayText(record);
  if (record.role !== 'assistant') return content;

  const embedded = new Set(
    Array.from(content.matchAll(TOOL_PLACEHOLDER_PATTERN), (match) => match[1]),
  );
  const anchors = toolCallIdsFromRecord(record)
    .filter((id) => !embedded.has(id))
    .map((id) => `[tool:status | call_id="${id}"]`);
  return [content, ...anchors].filter(Boolean).join('\n');
}

export function snapshotRecords(payload: FrontendSnapshotPayload): LedgerRecord[] {
  if (payload.ledger_records) return payload.ledger_records;
  if (payload.ledger) return payload.ledger;
  if (payload.ledger_delta?.records) return payload.ledger_delta.records;
  if (payload.ledger_delta?.record) return [payload.ledger_delta.record];
  return [];
}

export function dedupeRecords(records: LedgerRecord[]): LedgerRecord[] {
  const byKey = new Map<string, LedgerRecord>();
  for (const record of records) byKey.set(recordKey(record), record);
  return [...byKey.values()];
}

export function toolCallIdsFromRecord(record: LedgerRecord): string[] {
  const ids = Array.from(
    displayText(record).matchAll(TOOL_PLACEHOLDER_PATTERN),
    (match) => match[1],
  );
  const metadataIds = record.metadata?.extra?.tool_call_ids;
  if (Array.isArray(metadataIds)) {
    for (const id of metadataIds) {
      if (typeof id === 'string' && !ids.includes(id)) ids.push(id);
    }
  }
  const declarations = record.metadata?.extra?.tool_calls;
  if (Array.isArray(declarations)) {
    for (const call of declarations) {
      if (call && typeof call === 'object' && 'id' in call && typeof call.id === 'string' && !ids.includes(call.id)) ids.push(call.id);
    }
  }
  return ids;
}

export function toolCallFromRecord(record: LedgerRecord): ToolCallView | null {
  const subtype = record.metadata?.subtype;
  if (!subtype?.startsWith('tool_call_')) return null;

  const extra = record.metadata?.extra ?? {};
  const id = String(extra.call_id ?? record.metadata?.call_id ?? recordKey(record));
  const toolName = String(record.metadata?.tool_name ?? extra.tool_name ?? '');
  const title = String(
    record.metadata?.title ?? (toolName || recordText(record) || 'Tool call'),
  );
  const status = subtype === 'tool_call_interrupted_unknown' ||
      extra.status === 'interrupted_unknown' ||
      extra.status === 'recovery_interrupted'
    ? 'uncertain'
    : subtype === 'tool_call_failed'
      ? 'failed'
    : subtype === 'tool_call_finished'
      ? 'finished'
      : subtype === 'tool_call_permission_requested' ||
          extra.status === 'waiting_permission'
        ? 'waiting_permission'
          : 'running';

  return {
    id,
    title,
    status,
    toolName,
    command: record.metadata?.tool_command,
    detail: recordText(record),
  };
}

export function collectToolCalls(
  records: LedgerRecord[],
  previous: ToolCallView[] = [],
): ToolCallView[] {
  const calls = new Map(previous.map((call) => [call.id, call]));

  for (const record of records) {
    if (record.role === 'assistant') {
      const declarations = record.metadata?.extra?.tool_calls;
      if (Array.isArray(declarations)) {
        for (const declaration of declarations) {
          if (!declaration || typeof declaration !== 'object') continue;
          const value = declaration as Record<string, unknown>;
          const id = typeof value.id === 'string' ? value.id : '';
          const fn = value.function && typeof value.function === 'object'
            ? value.function as Record<string, unknown>
            : undefined;
          const toolName = typeof fn?.name === 'string' ? fn.name : '';
          if (!id || calls.has(id)) continue;
          calls.set(id, {
            id,
            title: toolName ? `Preparing ${toolName}` : 'Preparing tool call',
            status: 'placeholder',
            detail: '',
            toolName,
          });
        }
      }
      for (const id of toolCallIdsFromRecord(record)) {
        if (!calls.has(id)) {
          calls.set(id, {
            id,
            title: 'Preparing tool call',
            status: 'placeholder',
            detail: '',
            toolName: '',
          });
        }
      }
    }
    const call = toolCallFromRecord(record);
    if (call) calls.set(call.id, call);
  }

  return [...calls.values()].slice(-80);
}

export function isRenderableAssistantRecord(record: LedgerRecord): boolean {
  if (record.role !== 'assistant') return false;
  const visibleLines = displayText(record)
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(
      (line) =>
        line.length > 0 &&
        !/^\[tool:status\s*\|\s*call_id="[^"]+"\]$/.test(line),
    );
  return visibleLines.length > 0;
}
