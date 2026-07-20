export type LedgerRole = 'user' | 'assistant' | 'system' | 'tool' | 'gateway_message';

export type LedgerRecord = {
  record_id?: string | number;
  id?: string;
  role?: LedgerRole;
  content?: string;
  text?: string;
  created_at?: string;
  metadata?: {
    subtype?: string;
    title?: string;
    tool_name?: string;
    tool_command?: string;
    call_id?: string;
    success?: boolean;
    display_content?: string;
    extra?: Record<string, unknown>;
    [key: string]: unknown;
  };
};

export type FrontendSnapshotPayload = {
  revision?: number;
  conversation_state?: 'waiting' | 'thinking' | 'executing' | 'compacting' | 'stopping';
  ledger_records?: LedgerRecord[];
  ledger_delta?: {
    kind?: 'append' | 'replace';
    record?: LedgerRecord;
    records?: LedgerRecord[];
  };
  error?: string;
};

export function recordText(record: LedgerRecord): string {
  return record.content ?? record.text ?? '';
}

export function snapshotRecords(payload: FrontendSnapshotPayload): LedgerRecord[] {
  if (payload.ledger_records?.length) return payload.ledger_records;
  if (payload.ledger_delta?.records?.length) return payload.ledger_delta.records;
  if (payload.ledger_delta?.record) return [payload.ledger_delta.record];
  return [];
}
