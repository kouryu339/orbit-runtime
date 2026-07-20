import {
  collectToolCalls,
  dedupeRecords,
  displayText,
  isRenderableAssistantRecord,
  recordKey,
  snapshotRecords,
} from './records.js';
import type {
  ConversationAction,
  ConversationState,
  FrontendSnapshotPayload,
  LedgerRecord,
  PendingUserMessage,
} from './types.js';

export function createConversationState(
  conversationId: string | null = null,
): ConversationState {
  return {
    conversationId,
    connection: 'disconnected',
    initialized: false,
    snapshotReceived: false,
    revision: 0,
    eventSeq: 0,
    runtimeState: 'waiting',
    awaitingAssistantResponse: false,
    turnObservedRunning: false,
    turnObservedAssistant: false,
    records: [],
    pendingUserMessages: [],
    toolCalls: [],
    agents: [],
    pendingPermissions: [],
  };
}

export function conversationReducer(
  state: ConversationState,
  action: ConversationAction,
): ConversationState {
  switch (action.type) {
    case 'connection':
      return { ...state, connection: action.state };
    case 'conversation-created':
      if (state.conversationId && state.conversationId !== action.conversationId) {
        return {
          ...createConversationState(action.conversationId),
          connection: state.connection,
          initialized: true,
          eventSeq: action.eventSeq ?? 0,
        };
      }
      return {
        ...state,
        conversationId: action.conversationId,
        initialized: true,
        eventSeq: Math.max(state.eventSeq, action.eventSeq ?? 0),
      };
    case 'conversation-closed':
      if (
        action.conversationId &&
        state.conversationId &&
        action.conversationId !== state.conversationId
      ) {
        return state;
      }
      return {
        ...createConversationState(),
        connection: state.connection,
      };
    case 'snapshot':
      return applySnapshot(state, action.payload, action.eventSeq);
    case 'local-message-added':
      if (state.pendingUserMessages.some((item) => item.id === action.message.id)) {
        return state;
      }
      return {
        ...state,
        runtimeState: 'thinking',
        awaitingAssistantResponse: true,
        turnObservedRunning: false,
        turnObservedAssistant: false,
        lastError: undefined,
        pendingUserMessages: [...state.pendingUserMessages, action.message],
      };
    case 'local-message-accepted':
      return {
        ...state,
        pendingUserMessages: state.pendingUserMessages.map((message) =>
          message.id === action.id ? { ...message, state: 'accepted' } : message,
        ),
      };
    case 'local-message-failed':
      return {
        ...state,
        runtimeState: 'waiting',
        awaitingAssistantResponse: false,
        turnObservedRunning: false,
        turnObservedAssistant: false,
        lastError: action.error,
        pendingUserMessages: state.pendingUserMessages.map((message) =>
          message.id === action.id
            ? { ...message, state: 'failed', error: action.error }
            : message,
        ),
      };
    case 'clear-error':
      return { ...state, lastError: undefined };
    case 'reset':
      return {
        ...createConversationState(action.conversationId ?? null),
        connection: state.connection,
      };
  }
}

function applySnapshot(
  state: ConversationState,
  payload: FrontendSnapshotPayload,
  eventSeq = 0,
): ConversationState {
  const revision =
    typeof payload.revision === 'number' && Number.isFinite(payload.revision)
      ? payload.revision
      : state.revision;
  const snapshotSeq =
    typeof payload.conversation_event_seq === 'number' &&
    Number.isFinite(payload.conversation_event_seq)
      ? payload.conversation_event_seq
      : eventSeq;
  // Runtime event sequence and snapshot revision are independent clocks. Prefer
  // the event sequence whenever it exists; comparing a state delta's event
  // sequence to a ledger snapshot revision can otherwise discard a newer
  // `waiting` transition and leave the composer stuck in a running state.
  if (snapshotSeq > 0) {
    if (snapshotSeq < state.eventSeq) return state;
  } else if (revision < state.revision) {
    return state;
  }

  const incoming = snapshotRecords(payload);
  const pending = reconcilePendingMessages(state.pendingUserMessages, incoming);
  const records = mergeRecords(state.records, incoming, payload, pending.acknowledgedIds);
  const toolCalls = collectToolCalls(
    records,
    payload.ledger_delta?.kind === 'replace' ? [] : state.toolCalls,
  );
  const runtimeState = payload.conversation_state ?? state.runtimeState;
  const turnObservedRunning =
    state.turnObservedRunning || runtimeState !== 'waiting';
  const assistantOutputObserved = hasCurrentTurnAssistantOutput(state.records, incoming);
  const turnObservedAssistant =
    state.turnObservedAssistant ||
    assistantOutputObserved;
  const turnSettled =
    runtimeState === 'waiting' &&
    (turnObservedRunning || assistantOutputObserved);
  const pendingMessages =
    turnSettled && assistantOutputObserved
      ? pending.messages.filter((message) => message.state === 'failed')
      : pending.messages;
  const awaitingAssistantResponse =
    pendingMessages.some((message) => message.state !== 'failed') ||
    (state.awaitingAssistantResponse && !turnSettled) ||
    runtimeState !== 'waiting';

  return {
    ...state,
    initialized: state.initialized || Boolean(state.conversationId),
    snapshotReceived: true,
    revision: Math.max(state.revision, revision),
    eventSeq: Math.max(state.eventSeq, snapshotSeq),
    runtimeState,
    awaitingAssistantResponse,
    turnObservedRunning: turnSettled ? false : turnObservedRunning,
    turnObservedAssistant: turnSettled ? false : turnObservedAssistant,
    records,
    pendingUserMessages: pendingMessages,
    toolCalls,
    agents: payload.agents ?? state.agents,
    plan: payload.plan ?? state.plan,
    pendingPermissions: payload.pending_permissions ?? payload.pendingPermissions ?? state.pendingPermissions,
    lastError: payload.error ?? state.lastError,
  };
}

function hasCurrentTurnAssistantOutput(
  current: LedgerRecord[],
  incoming: LedgerRecord[],
): boolean {
  const currentByKey = new Map(
    current.map((record) => [recordKey(record), displayText(record)]),
  );
  return incoming.some((record) => {
    if (!isRenderableAssistantRecord(record)) return false;
    const previous = currentByKey.get(recordKey(record));
    return previous === undefined || previous !== displayText(record);
  });
}

function mergeRecords(
  current: LedgerRecord[],
  incoming: LedgerRecord[],
  payload: FrontendSnapshotPayload,
  acknowledgedIds: Set<string>,
): LedgerRecord[] {
  if (!incoming.length) return current;
  const withoutAcknowledged = current.filter(
    (record) =>
      typeof record.record_id !== 'string' ||
      !acknowledgedIds.has(record.record_id),
  );

  if (payload.ledger_records || payload.ledger || payload.ledger_delta?.kind === 'replace') {
    return dedupeRecords(incoming);
  }
  return dedupeRecords([...withoutAcknowledged, ...incoming]);
}

function reconcilePendingMessages(
  pending: PendingUserMessage[],
  incoming: LedgerRecord[],
): { messages: PendingUserMessage[]; acknowledgedIds: Set<string> } {
  const remaining = [...pending];
  const acknowledgedIds = new Set<string>();

  for (const record of incoming) {
    if (record.role !== 'user') continue;
    const match = findPendingMatch(remaining, displayText(record));
    for (const message of match) {
      acknowledgedIds.add(message.id);
      const index = remaining.findIndex((item) => item.id === message.id);
      if (index >= 0) remaining.splice(index, 1);
    }
  }
  return { messages: remaining, acknowledgedIds };
}

function findPendingMatch(
  pending: PendingUserMessage[],
  content: string,
): PendingUserMessage[] {
  const exact = pending.find((message) => message.content === content);
  if (exact) return [exact];

  const matched: PendingUserMessage[] = [];
  const parts: string[] = [];
  for (const message of pending) {
    matched.push(message);
    parts.push(message.content.trim());
    const combined = parts.join('\n\n');
    if (combined === content) return matched;
    if (!content.startsWith(`${combined}\n\n`)) break;
  }
  return [];
}

export function createPendingUserMessage(
  id: string,
  content: string,
  createdAt = new Date().toISOString(),
): PendingUserMessage {
  return { id, content, createdAt, state: 'sending' };
}
