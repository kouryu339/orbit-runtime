import { describe, it, expect } from "vitest";
import { WorkerConversationTransport } from "../src/transport/worker.js";

describe("Worker transport shared by Lit and Svelte", () => {
  it("subscribes before requesting state and filters other conversations", async () => {
    let listener: (event: any) => void = () => {};
    let unsubscribed = false;
    const calls: string[] = [];
    const runtime = {
      subscribe(handler: (event: any) => void) {
        listener = handler;
        return () => {
          unsubscribed = true;
        };
      },
      async invoke<T = any>(type: string, payload?: any): Promise<T> {
        calls.push(type);
        if (type === "conversation.materialize")
          listener({
            type: "frontend:state_snapshot",
            conversation_id: payload.conversation_id,
            event_seq: 1,
            payload: { revision: 1, agents: [] },
          });
        return {} as T;
      },
    };
    const transport = new WorkerConversationTransport(runtime);
    const events: any[] = [];
    const connection = await transport.connect(
      { conversationId: "a" },
      {
        event: (event) => events.push(event),
        connection: () => {},
        error: () => {},
      },
    );
    listener({
      type: "frontend:state_snapshot",
      conversation_id: "b",
      payload: {},
    });
    expect(calls).toEqual(["conversation.materialize"]);
    expect(events.map((event) => event.type)).toEqual([
      "conversation-created",
      "state-snapshot",
    ]);
    connection.disconnect();
    expect(unsubscribed).toBe(true);
  });
  it("uses admission results for send, pause and resume without synthesizing messages", async () => {
    const calls: any[] = [];
    const transport = new WorkerConversationTransport({
      subscribe: () => () => {},
      async invoke<T = any>(...args: any[]): Promise<T> {
        calls.push(args);
        return { decision: "accepted", command_id: "c" } as T;
      },
    });
    expect(
      await transport.send({
        conversationId: "a",
        clientMessageId: "m",
        content: "hello",
      }),
    ).toEqual({ accepted: true, commandId: "c" });
    expect(calls[0]).toEqual([
      "conversation.send_message",
      { conversation_id: "a", content: "hello" },
      { commandId: "m" },
    ]);
    expect(await transport.pause({ conversationId: 'a' })).toMatchObject({ accepted: true });
    expect(await transport.resume({ conversationId: 'a' })).toMatchObject({ accepted: true });
    expect(calls.slice(1)).toEqual([
      ['conversation.pause', { conversation_id: 'a' }],
      ['conversation.resume', { conversation_id: 'a' }],
    ]);
    expect((transport as any).imageInput).toBeUndefined();
  });
  it('returns Runtime rejection instead of treating an unavailable resume as accepted', async () => {
    const transport = new WorkerConversationTransport({
      subscribe: () => () => {},
      async invoke<T>(): Promise<T> {
        return { command_id: 'c', decision: 'rejected', reason: 'not_paused' } as T;
      },
    });
    expect(await transport.resume({ conversationId: 'a' })).toEqual({
      accepted: false, commandId: 'c', rejectReason: 'not_paused',
    });
  });
});
