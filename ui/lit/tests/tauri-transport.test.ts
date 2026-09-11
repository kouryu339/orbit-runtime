import { describe, expect, it, vi } from 'vitest';
import { TauriConversationTransport } from '../src/transport/tauri.js';

describe('TauriConversationTransport', () => {
  it('prepares, hydrates, restores pending messages, and releases listeners', async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    const unlistenRuntime = vi.fn();
    const unlistenPending = vi.fn();
    const listen = vi.fn(async (
      event: string,
      handler: (event: { payload: unknown }) => void,
    ) => {
      handlers.set(event, handler);
      if (event === 'ai-frontend-event') return unlistenRuntime;
      return unlistenPending;
    });
    const invoke = vi.fn(async (command: string) => {
      if (command === 'ai_prepare_conversation') {
        return { conversation_id: 'sunwoo-1' };
      }
      if (command === 'ai_runtime_snapshot') {
        return {
          type: 'frontend:state_snapshot',
          conversation_id: 'sunwoo-1',
          payload: { revision: 1, conversation_state: 'waiting' },
        };
      }
      if (command === 'ai_pending_messages') {
        return [{
          message_id: 'pending-1',
          conversation_id: 'sunwoo-1',
          content: 'queued',
        }];
      }
      return { accepted: true };
    });
    const events: unknown[] = [];
    const transport = new TauriConversationTransport({
      invoke: invoke as never,
      listen: listen as never,
    });

    const connection = await transport.connect({}, {
      event: (event) => events.push(event),
      connection: vi.fn(),
      error: vi.fn(),
    });

    expect(events).toEqual([
      expect.objectContaining({
        type: 'conversation-created',
        conversationId: 'sunwoo-1',
      }),
      expect.objectContaining({
        type: 'state-snapshot',
        conversationId: 'sunwoo-1',
      }),
      expect.objectContaining({
        type: 'pending-user-message',
        messageId: 'pending-1',
      }),
    ]);

    await connection.disconnect();
    expect(unlistenRuntime).toHaveBeenCalledOnce();
    expect(unlistenPending).toHaveBeenCalledOnce();
  });

  it('returns a permission decision with conversation and tool call ids', async () => {
    const invoke = vi.fn(async () => ({ resolved: true }));
    const transport = new TauriConversationTransport({
      invoke: invoke as never,
      listen: vi.fn() as never,
    });

    await expect(transport.resolveToolPermission({
      conversationId: 'conversation-1',
      toolCallId: 'call-1',
      decision: 'deny',
    })).resolves.toMatchObject({ accepted: true });
    expect(invoke).toHaveBeenCalledWith('ai_resolve_tool_permission', {
      args: {
        conversation_id: 'conversation-1',
        tool_call_id: 'call-1',
        decision: 'deny',
      },
    });
  });

  it('maps ledger deltas to message snapshots and forwards state/telemetry events', async () => {
    const handlers = new Map<string, (event: { payload: unknown }) => void>();
    const listen = vi.fn(async (
      event: string,
      handler: (event: { payload: unknown }) => void,
    ) => {
      handlers.set(event, handler);
      return vi.fn();
    });
    const invoke = vi.fn(async (command: string) => {
      if (command === 'ai_prepare_conversation') return { conversation_id: 'sunwoo-1' };
      if (command === 'ai_runtime_snapshot') return {};
      if (command === 'ai_pending_messages') return [];
      return { accepted: true };
    });
    const events: unknown[] = [];
    const transport = new TauriConversationTransport({
      invoke: invoke as never,
      listen: listen as never,
    });

    await transport.connect({}, {
      event: (event) => events.push(event),
      connection: vi.fn(),
      error: vi.fn(),
    });

    const runtimeHandler = handlers.get('ai-frontend-event');
    expect(runtimeHandler).toBeDefined();
    runtimeHandler?.({
      payload: {
        type: 'conversation.ledger_delta',
        conversation_id: 'sunwoo-1',
        event_seq: 2,
        payload: {
          conversation_id: 'sunwoo-1',
          op: 'append',
          record: { record_id: 'r1', role: 'assistant', content: 'hello' },
        },
      },
    });
    runtimeHandler?.({
      payload: {
        type: 'conversation.state_delta',
        conversation_id: 'sunwoo-1',
        event_seq: 3,
        payload: { conversation_id: 'sunwoo-1', op: 'agent_plan.set' },
      },
    });
    expect(events).toContainEqual(expect.objectContaining({
      type: 'state-snapshot',
      eventSeq: 2,
      payload: expect.objectContaining({
        ledger_delta: expect.objectContaining({
          kind: 'append',
          record: expect.objectContaining({ record_id: 'r1' }),
        }),
      }),
    }));
    expect(events).toContainEqual(expect.objectContaining({
      type: 'transport-extension',
      extension: expect.objectContaining({
        namespace: 'org.agent-runtime.state-delta',
        kind: 'conversation.state_delta',
      }),
    }));
  });

  it('wraps default command payloads in the Tauri args parameter', async () => {
    const invoke = vi.fn(async (command: string) => {
      if (command === 'ai_prepare_conversation') return { conversation_id: 'c-1' };
      if (command === 'ai_runtime_snapshot') return {};
      if (command === 'ai_pending_messages') return [];
      return { accepted: true };
    });
    const transport = new TauriConversationTransport({
      invoke: invoke as never,
      listen: vi.fn(async () => vi.fn()) as never,
    });

    const connection = await transport.connect({}, {
      event: vi.fn(),
      connection: vi.fn(),
      error: vi.fn(),
    });
    const conversationId = connection.conversationId!;
    await transport.send({
      conversationId,
      content: 'hello',
      clientMessageId: 'client-1',
    });
    await transport.pause({ conversationId });

    expect(invoke).toHaveBeenCalledWith('ai_runtime_snapshot', {
      args: { conversationId: 'c-1' },
    });
    expect(invoke).toHaveBeenCalledWith('ai_send_message', {
      args: {
        conversationId: 'c-1',
        content: 'hello',
        clientMessageId: 'client-1',
        metadata: undefined,
      },
    });
    expect(invoke).toHaveBeenCalledWith('ai_pause_conversation', {
      args: { conversationId: 'c-1' },
    });
  });

  it('does not poll snapshots after send; live events remain the render authority', async () => {
    vi.useFakeTimers();
    try {
      let snapshotRead = 0;
      const invoke = vi.fn(async (command: string) => {
        if (command === 'ai_prepare_conversation') return { conversation_id: 'c-1' };
        if (command === 'ai_runtime_snapshot') {
          snapshotRead += 1;
          const conversationState = snapshotRead === 1
            ? 'waiting'
            : snapshotRead === 2
              ? 'executing'
              : 'waiting';
          return {
            type: 'frontend:state_snapshot',
            conversation_id: 'c-1',
            payload: {
              revision: snapshotRead,
              conversation_state: conversationState,
              ledger_records: snapshotRead >= 3
                ? [{ record_id: 1, role: 'assistant', content: 'done' }]
                : [],
            },
          };
        }
        if (command === 'ai_pending_messages') return [];
        return { accepted: true };
      });
      const transport = new TauriConversationTransport({
        invoke: invoke as never,
        listen: vi.fn(async () => vi.fn()) as never,
      });
      await transport.connect({}, {
        event: vi.fn(),
        connection: vi.fn(),
        error: vi.fn(),
      });

      await transport.send({
        conversationId: 'c-1',
        content: 'run',
        clientMessageId: 'client-1',
      });
      await vi.advanceTimersByTimeAsync(750);
      await vi.advanceTimersByTimeAsync(1_500);

      expect(snapshotRead).toBe(1);
      expect(invoke).toHaveBeenCalledTimes(4);
    } finally {
      vi.useRealTimers();
    }
  });
});
