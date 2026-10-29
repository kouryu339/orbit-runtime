import { TRANSPORT_CONTRACT, } from "../host/types.js";
import { defaultEnvelopeEvents } from "./http-sse.js";
export class WorkerConversationTransport {
    runtime;
    contract = TRANSPORT_CONTRACT;
    id = "browser-worker";
    constructor(runtime) {
        this.runtime = runtime;
    }
    async connect(context, handlers) {
        const unsubscribe = this.runtime.subscribe((event) => {
            if (context.conversationId &&
                event.conversation_id !== context.conversationId)
                return;
            for (const projected of defaultEnvelopeEvents(event))
                handlers.event(projected);
        });
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
        }
        catch (error) {
            unsubscribe();
            context.signal?.removeEventListener("abort", unsubscribe);
            throw error;
        }
    }
    async send(request) {
        request.signal?.throwIfAborted();
        if (request.parts?.some((part) => part.type === "image"))
            throw new Error("Image input requires a browser asset adapter");
        const result = await this.runtime.invoke("conversation.send_message", { conversation_id: request.conversationId, content: request.content }, { commandId: request.clientMessageId });
        return {
            accepted: result.decision === "accepted",
            commandId: result.command_id,
        };
    }
    async pause(request) {
        request.signal?.throwIfAborted();
        const result = await this.runtime.invoke("conversation.pause", {
            conversation_id: request.conversationId,
        });
        return { accepted: result.accepted === true };
    }
    async close(request) {
        request.signal?.throwIfAborted();
        await this.runtime.invoke("conversation.close", {
            conversation_id: request.conversationId,
        });
        return { accepted: true };
    }
    async requestSnapshot(request) {
        request.signal?.throwIfAborted();
        await this.runtime.invoke("conversation.materialize", {
            conversation_id: request.conversationId,
        });
    }
    async resolveToolPermission(request) {
        request.signal?.throwIfAborted();
        const result = await this.runtime.invoke("conversation.resolve_tool_permission", {
            conversation_id: request.conversationId,
            tool_call_id: request.toolCallId,
            decision: request.decision,
        });
        return { accepted: result.resolved === true };
    }
}
//# sourceMappingURL=worker.js.map