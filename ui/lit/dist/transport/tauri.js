import { TRANSPORT_CONTRACT, } from '../host/types.js';
import { defaultEnvelopeEvents } from './http-sse.js';
export class TauriConversationTransport {
    contract = TRANSPORT_CONTRACT;
    id;
    config;
    unlisten = new Set();
    handlers = null;
    conversationId = null;
    constructor(config) {
        this.config = config;
        this.id = config.id ?? 'tauri-events';
    }
    async connect(context, handlers) {
        this.disconnect();
        this.handlers = handlers;
        handlers.connection('connected');
        const runtimeEvent = this.config.events?.runtime ?? 'ai-frontend-event';
        const unlistenRuntime = await this.config.listen(runtimeEvent, (event) => this.handleEnvelope(event.payload));
        this.unlisten.add(unlistenRuntime);
        const pendingEvent = this.config.events?.pendingMessage ?? 'ai-pending-user-message';
        const unlistenPending = await this.config.listen(pendingEvent, (event) => this.handlePendingMessage(event.payload));
        this.unlisten.add(unlistenPending);
        this.conversationId = context.conversationId ?? null;
        if (!this.conversationId) {
            const prepare = this.config.commands?.prepare ?? 'ai_prepare_conversation';
            const args = await this.config.prepareArgs?.(context) ?? {};
            const created = await this.config.invoke(prepare, args);
            this.conversationId =
                created.conversation_id ?? created.conversationId ?? null;
        }
        if (!this.conversationId) {
            throw new Error('Tauri prepare command did not return a conversation ID.');
        }
        handlers.event({
            type: 'conversation-created',
            conversationId: this.conversationId,
        });
        await this.loadSnapshot();
        await this.loadPendingMessages();
        const connectedConversationId = this.conversationId;
        return {
            conversationId: connectedConversationId,
            disconnect: () => this.disconnect(),
        };
    }
    async send(request) {
        const command = this.config.commands?.send ?? 'ai_send_message';
        const args = await this.config.sendArgs?.(request) ?? {
            args: {
                conversationId: request.conversationId,
                content: request.content,
                clientMessageId: request.clientMessageId,
                metadata: request.metadata,
            },
        };
        const value = await this.config.invoke(command, args);
        const result = normalizeCommandResult(value);
        return result;
    }
    async requestSnapshot(request) {
        if (!this.handlers || request.conversationId !== this.conversationId)
            return;
        await this.loadSnapshot();
    }
    async pause(request) {
        const command = this.config.commands?.pause ?? 'ai_pause_conversation';
        const args = await this.commandArgs(request.conversationId);
        return normalizeCommandResult(await this.config.invoke(command, args));
    }
    async close(request) {
        const command = this.config.commands?.close ?? 'ai_close_conversation';
        const args = await this.commandArgs(request.conversationId);
        const result = normalizeCommandResult(await this.config.invoke(command, args));
        if (request.conversationId === this.conversationId) {
            this.conversationId = null;
        }
        return result;
    }
    async resolveToolPermission(request) {
        const command = this.config.commands?.resolveToolPermission ??
            'ai_resolve_tool_permission';
        const args = await this.config.permissionArgs?.(request) ?? {
            args: {
                conversation_id: request.conversationId,
                tool_call_id: request.toolCallId,
                decision: request.decision,
            },
        };
        return normalizeCommandResult(await this.config.invoke(command, args));
    }
    disconnect() {
        for (const unlisten of this.unlisten)
            unlisten();
        this.unlisten.clear();
        this.handlers = null;
    }
    async loadSnapshot() {
        const command = this.config.commands?.snapshot ?? 'ai_runtime_snapshot';
        const args = await this.config.snapshotArgs?.(this.conversationId) ??
            (this.conversationId ? { args: { conversationId: this.conversationId } } : {});
        const value = await this.config.invoke(command, args);
        if (!value || typeof value !== 'object')
            return;
        const envelope = value;
        if (typeof envelope.type === 'string') {
            this.handleEnvelope(envelope);
            return;
        }
        this.handlers?.event({
            type: 'state-snapshot',
            conversationId: this.conversationId ?? undefined,
            payload: value,
        });
    }
    async loadPendingMessages() {
        const command = this.config.commands?.pendingMessages ?? 'ai_pending_messages';
        const messages = await this.config.invoke(command);
        for (const message of messages ?? [])
            this.handlePendingMessage(message);
    }
    handleEnvelope(envelope) {
        const events = this.config.mapEvent?.(envelope) ??
            defaultEnvelopeEvents(envelope);
        for (const event of events) {
            if (event.type === 'conversation-created' &&
                !this.conversationId) {
                this.conversationId = event.conversationId;
            }
            this.handlers?.event(event);
        }
    }
    handlePendingMessage(message) {
        const conversationId = message.conversation_id ?? message.conversationId;
        const messageId = message.message_id ?? message.messageId;
        if (!conversationId || !messageId || !message.content)
            return;
        if (this.conversationId && conversationId !== this.conversationId)
            return;
        this.handlers?.event({
            type: 'pending-user-message',
            conversationId,
            messageId,
            content: message.content,
            createdAt: message.created_at ?? message.createdAt,
        });
    }
    async commandArgs(conversationId) {
        return await this.config.commandArgs?.(conversationId) ?? {
            args: { conversationId },
        };
    }
}
function normalizeCommandResult(value) {
    if (!value || typeof value !== 'object')
        return { accepted: true };
    const record = value;
    const accepted = record.accepted !== false && record.resolved !== false;
    return {
        accepted,
        commandId: typeof record.command_id === 'string'
            ? record.command_id
            : typeof record.commandId === 'string'
                ? record.commandId
                : record.resolved === false
                    ? 'Permission request is no longer pending.'
                    : undefined,
        rejectReason: typeof record.reject_reason === 'string'
            ? record.reject_reason
            : typeof record.rejectReason === 'string'
                ? record.rejectReason
                : undefined,
        metadata: record,
    };
}
//# sourceMappingURL=tauri.js.map