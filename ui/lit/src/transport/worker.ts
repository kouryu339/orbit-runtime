import {
  TRANSPORT_CONTRACT,
  type ConversationTransport,
  type ConversationConnectContext,
  type ConversationTransportHandlers,
  type SendMessageRequest,
  type ConversationCommandRequest,
  type ResolveToolPermissionRequest,
} from "../host/types.js";
import type { RuntimeEventEnvelope } from "../protocol/types.js";
import { defaultEnvelopeEvents } from "./http-sse.js";

/** Structural contract: both UI implementations can use @orbit-runtime/browser. */
export interface WorkerRuntimeClient {
  invoke<T = any>(
    type: string,
    payload?: any,
    options?: { commandId?: string },
  ): Promise<T>;
  subscribe(listener: (event: any) => void): () => void;
}
export class WorkerConversationTransport implements ConversationTransport {
  readonly contract = TRANSPORT_CONTRACT;
  readonly id = "browser-worker";
  constructor(private readonly runtime: WorkerRuntimeClient) {}
  async connect(
    context: ConversationConnectContext,
    handlers: ConversationTransportHandlers,
  ) {
    const unsubscribe = this.runtime.subscribe(
      (event: RuntimeEventEnvelope) => {
        if (
          context.conversationId &&
          event.conversation_id !== context.conversationId
        )
          return;
        for (const projected of defaultEnvelopeEvents(event))
          handlers.event(projected);
      },
    );
    try {
      if (context.signal?.aborted)
        throw new DOMException("Connection aborted", "AbortError");
      context.signal?.addEventListener("abort", unsubscribe, { once: true });
      if (context.conversationId) {
        handlers.event({
          type: "conversation-created",
          conversationId: context.conversationId,
        });
        await this.runtime.invoke("conversation.materialize", {
          conversation_id: context.conversationId,
        });
      }
      context.signal?.throwIfAborted();
      handlers.connection("connected");
      return {
        conversationId: context.conversationId ?? undefined,
        disconnect: () => {
          unsubscribe();
          context.signal?.removeEventListener("abort", unsubscribe);
          handlers.connection("disconnected");
        },
      };
    } catch (error) {
      unsubscribe();
      context.signal?.removeEventListener("abort", unsubscribe);
      throw error;
    }
  }
  async send(request: SendMessageRequest) {
    request.signal?.throwIfAborted();
    if (request.parts?.some((part) => part.type === "image"))
      throw new Error("Image input requires a browser asset adapter");
    const result = await this.runtime.invoke(
      "conversation.send_message",
      { conversation_id: request.conversationId, content: request.content },
      { commandId: request.clientMessageId },
    );
    return {
      accepted: result.decision === "accepted",
      commandId: result.command_id,
    };
  }
  async pause(request: ConversationCommandRequest) {
    request.signal?.throwIfAborted();
    const result = await this.runtime.invoke("conversation.pause", {
      conversation_id: request.conversationId,
    });
    return { accepted: result.accepted === true };
  }
  async close(request: ConversationCommandRequest) {
    request.signal?.throwIfAborted();
    await this.runtime.invoke("conversation.close", {
      conversation_id: request.conversationId,
    });
    return { accepted: true };
  }
  async requestSnapshot(request: ConversationCommandRequest) {
    request.signal?.throwIfAborted();
    await this.runtime.invoke("conversation.materialize", {
      conversation_id: request.conversationId,
    });
  }
  async resolveToolPermission(request: ResolveToolPermissionRequest) {
    request.signal?.throwIfAborted();
    const result = await this.runtime.invoke(
      "conversation.resolve_tool_permission",
      {
        conversation_id: request.conversationId,
        tool_call_id: request.toolCallId,
        decision: request.decision,
      },
    );
    return { accepted: result.resolved === true };
  }
}
