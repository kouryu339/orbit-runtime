import { describe, expect, it } from 'vitest';
import {
  conversationReducer,
  createConversationState,
  createPendingUserMessage,
} from '../src/protocol/index.js';

describe('conversationReducer', () => {
  it('keeps the composer disabled until an authoritative waiting snapshot arrives', () => {
    let state = createConversationState();
    state = conversationReducer(state, {
      type: 'conversation-created',
      conversationId: 'editor',
    });
    expect(state.initialized).toBe(true);
    expect(state.snapshotReceived).toBe(false);
    expect(state.runtimeState).toBe('waiting');

    state = conversationReducer(state, {
      type: 'snapshot',
      payload: { revision: 1, conversation_state: 'waiting' },
    });
    expect(state.snapshotReceived).toBe(true);
    expect(state.runtimeState).toBe('waiting');
  });

  it('uses conversation_state as the authoritative running-state signal', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: { revision: 1, conversation_state: 'thinking' },
    });
    expect(state.runtimeState).toBe('thinking');
    expect(state.awaitingAssistantResponse).toBe(true);

    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'waiting',
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'a1', role: 'assistant', content: 'done' },
        },
      },
    });
    expect(state.runtimeState).toBe('waiting');
    expect(state.awaitingAssistantResponse).toBe(false);
  });

  it('does not settle a new turn on a delayed waiting snapshot', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'local-message-added',
      message: createPendingUserMessage('local-1', 'hello'),
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 1,
        conversation_state: 'waiting',
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'u1', role: 'user', content: 'hello' },
        },
      },
    });
    expect(state.awaitingAssistantResponse).toBe(true);

    state = conversationReducer(state, {
      type: 'snapshot',
      payload: { revision: 2, conversation_state: 'thinking' },
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 3,
        conversation_state: 'waiting',
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'a1', role: 'assistant', content: 'done' },
        },
      },
    });
    expect(state.awaitingAssistantResponse).toBe(false);
  });

  it('settles when a started turn returns to waiting without assistant text', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'local-message-added',
      message: createPendingUserMessage('local-1', 'hello'),
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 1,
        conversation_state: 'thinking',
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'u1', role: 'user', content: 'hello' },
        },
      },
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: { revision: 2, conversation_state: 'waiting' },
    });

    expect(state.awaitingAssistantResponse).toBe(false);
  });

  it('does not treat unchanged historical assistant records as current output', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: {
        revision: 1,
        conversation_state: 'waiting',
        ledger_records: [{
          record_id: 'old-assistant',
          role: 'assistant',
          content: 'Earlier answer',
        }],
      },
    });
    state = conversationReducer(state, {
      type: 'local-message-added',
      message: createPendingUserMessage('local-1', 'hello'),
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'thinking',
        ledger_records: [
          {
            record_id: 'old-assistant',
            role: 'assistant',
            content: 'Earlier answer',
          },
          { record_id: 'u1', role: 'user', content: 'hello' },
        ],
      },
    });

    expect(state.turnObservedAssistant).toBe(false);
  });

  it('ignores stale revisions', () => {
    const current = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: {
        revision: 5,
        conversation_state: 'waiting',
        ledger_records: [{ record_id: 'a', role: 'assistant', content: 'new' }],
      },
    });
    const stale = conversationReducer(current, {
      type: 'snapshot',
      payload: {
        revision: 4,
        conversation_state: 'thinking',
        ledger_records: [{ record_id: 'b', role: 'assistant', content: 'old' }],
      },
    });
    expect(stale).toBe(current);
  });

  it('accepts a newer runtime event even when its snapshot revision is lower', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      eventSeq: 40,
      payload: {
        revision: 100,
        conversation_event_seq: 40,
        conversation_state: 'executing',
      },
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      eventSeq: 41,
      payload: {
        revision: 41,
        conversation_event_seq: 41,
        conversation_state: 'waiting',
      },
    });

    expect(state.runtimeState).toBe('waiting');
    expect(state.eventSeq).toBe(41);
    expect(state.revision).toBe(100);
  });

  it('ignores a non-numeric compatibility revision without corrupting state', () => {
    const state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: {
        revision: 'poll' as unknown as number,
        conversation_state: 'waiting',
        ledger_records: [{ record_id: 'a', role: 'assistant', content: 'ok' }],
      },
    });
    expect(state.revision).toBe(0);
    expect(state.snapshotReceived).toBe(true);
    expect(state.runtimeState).toBe('waiting');
    expect(state.records).toHaveLength(1);
  });

  it('replaces authoritative records on a replace snapshot', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: {
        revision: 1,
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'old', role: 'assistant', content: 'old' },
        },
      },
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        ledger_delta: {
          kind: 'replace',
          records: [{ record_id: 'new', role: 'assistant', content: 'new' }],
        },
      },
    });
    expect(state.records.map((record) => record.record_id)).toEqual(['new']);
  });

  it('keeps records when a state-only snapshot advances runtime fields', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: {
        revision: 1,
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'a1', role: 'assistant', content: 'kept' },
        },
      },
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'waiting',
        agents: [{ agent_id: 'boss', status: 'suspended' }],
      },
    });

    expect(state.records.map((record) => record.record_id)).toEqual(['a1']);
    expect(state.agents).toEqual([{ agent_id: 'boss', status: 'suspended' }]);
    expect(state.runtimeState).toBe('waiting');
  });

  it('treats runtime export ledger as an authoritative record list', () => {
    const state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: {
        revision: 1,
        conversation_state: 'waiting',
        ledger: [{ record_id: 'a1', role: 'assistant', content: 'from export' }],
      },
    });

    expect(state.records.map((record) => record.record_id)).toEqual(['a1']);
  });

  it('deduplicates replayed append records after reconnect', () => {
    const payload = {
      revision: 1,
      ledger_delta: {
        kind: 'append' as const,
        record: { record_id: 'same', role: 'assistant' as const, content: 'hello' },
      },
    };
    let state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload,
    });
    state = conversationReducer(state, { type: 'snapshot', payload });
    expect(state.records).toHaveLength(1);
  });

  it('reconciles an optimistic user message with its authoritative echo', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'local-message-added',
      message: createPendingUserMessage('local-1', 'hello'),
    });
    expect(state.awaitingAssistantResponse).toBe(true);

    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 1,
        conversation_state: 'thinking',
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'server-1', role: 'user', content: 'hello' },
        },
      },
    });
    expect(state.pendingUserMessages).toHaveLength(0);
    expect(state.records.map((record) => record.record_id)).toEqual(['server-1']);
  });

  it('stops waiting when a renderable assistant answer arrives', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'local-message-added',
      message: createPendingUserMessage('local-1', 'hello'),
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 1,
        conversation_state: 'thinking',
        ledger_records: [{ record_id: 'u1', role: 'user', content: 'hello' }],
      },
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'waiting',
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'a1', role: 'assistant', content: 'done' },
        },
      },
    });
    expect(state.awaitingAssistantResponse).toBe(false);
    expect(state.runtimeState).toBe('waiting');
  });

  it('releases an unacknowledged local message after a waiting assistant answer', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'local-message-added',
      message: createPendingUserMessage('local-1', 'hello'),
    });
    state = conversationReducer(state, {
      type: 'local-message-accepted',
      id: 'local-1',
    });
    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 1,
        conversation_state: 'waiting',
        ledger_records: [{
          record_id: 'a1',
          role: 'assistant',
          content: 'Which scenario should I test?',
        }],
      },
    });

    expect(state.pendingUserMessages).toEqual([]);
    expect(state.awaitingAssistantResponse).toBe(false);
    expect(state.runtimeState).toBe('waiting');
  });

  it('merges tool placeholder, running, and finished records by call id', () => {
    const state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: {
        revision: 1,
        ledger_records: [
          {
            record_id: 'a1',
            role: 'assistant',
            content: '[tool:status | call_id="call-1"]',
          },
          {
            record_id: 't1',
            role: 'gateway_message',
            content: 'Reading',
            metadata: {
              subtype: 'tool_call_started',
              title: 'Read file',
              extra: { call_id: 'call-1' },
            },
          },
          {
            record_id: 't2',
            role: 'tool',
            content: 'ok',
            metadata: {
              subtype: 'tool_call_finished',
              title: 'Read file',
              extra: { call_id: 'call-1' },
            },
          },
        ],
      },
    });
    expect(state.toolCalls).toEqual([
      expect.objectContaining({ id: 'call-1', status: 'finished', detail: 'ok' }),
    ]);
  });

  it('merges tool permission requests into the same tool call status line', () => {
    let state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: {
        revision: 1,
        ledger_records: [
          {
            record_id: 'a1',
            role: 'assistant',
            content: '[tool:status | call_id="call-1"]',
          },
          {
            record_id: 't1',
            role: 'gateway_message',
            content: 'Waiting for permission: WriteFile',
            metadata: {
              subtype: 'tool_call_permission_requested',
              title: 'Write file',
              extra: { call_id: 'call-1', status: 'waiting_permission' },
            },
          },
        ],
      },
    });
    expect(state.toolCalls).toEqual([
      expect.objectContaining({
        id: 'call-1',
        status: 'waiting_permission',
        title: 'Write file',
      }),
    ]);

    state = conversationReducer(state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        ledger_delta: {
          kind: 'append',
          record: {
            record_id: 't2',
            role: 'gateway_message',
            content: 'Running',
            metadata: {
              subtype: 'tool_call_started',
              title: 'Write file',
              extra: { call_id: 'call-1' },
            },
          },
        },
      },
    });
    expect(state.toolCalls).toEqual([
      expect.objectContaining({ id: 'call-1', status: 'running' }),
    ]);
  });

  it('isolates state when the conversation id changes', () => {
    let state = conversationReducer(createConversationState('first'), {
      type: 'snapshot',
      payload: {
        revision: 3,
        conversation_state: 'waiting',
        ledger_records: [{ record_id: 'a1', role: 'assistant', content: 'first' }],
      },
    });
    state = conversationReducer(state, {
      type: 'conversation-created',
      conversationId: 'second',
    });
    expect(state.conversationId).toBe('second');
    expect(state.records).toEqual([]);
    expect(state.revision).toBe(0);
    expect(state.runtimeState).toBe('waiting');
  });

  it('tracks pending tool permissions from authoritative snapshots', () => {
    const permission = {
      conversation_id: 'c1',
      tool_call_id: 'call-1',
      agent_id: 'boss',
      tool_name: 'WriteFile',
      display_name: 'Write file',
      effect: 'controlled_change' as const,
      arguments: { path: 'report.md' },
      turn_id: 2,
      created_at: '2026-06-22T00:00:00Z',
    };
    let state = conversationReducer(createConversationState('c1'), {
      type: 'snapshot',
      payload: { revision: 1, pending_permissions: [permission] },
    });
    expect(state.pendingPermissions).toEqual([permission]);

    state = conversationReducer(state, {
      type: 'snapshot',
      payload: { revision: 2, pending_permissions: [] },
    });
    expect(state.pendingPermissions).toEqual([]);
  });
});
