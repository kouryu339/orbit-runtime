import { type CommandResult, type ConversationConnectContext, type ConversationConnection, type ConversationTransport, type ConversationTransportEvent, type ConversationTransportHandlers, type SendMessageRequest, type SendResult, type ResolveToolPermissionRequest } from '../host/types.js';
import type { RuntimeEventEnvelope } from '../protocol/types.js';
export type HttpSseTransportConfig = {
    id?: string;
    baseUrl?: string;
    contextPath?: string;
    eventsPath?: string;
    sendPath?: string;
    pausePath?: string;
    closePath?: string;
    permissionPath?: string;
    query?: Record<string, string | undefined>;
    headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>);
    fetch?: typeof globalThis.fetch;
    eventSourceFactory?: (url: string) => EventSource;
    mapContext?: (value: unknown, context: ConversationConnectContext) => ConversationTransportEvent[];
    mapEvent?: (envelope: RuntimeEventEnvelope) => ConversationTransportEvent[];
    createSendBody?: (request: SendMessageRequest) => unknown;
};
export declare class HttpSseConversationTransport implements ConversationTransport {
    readonly contract: "agent-conversation-transport/v1";
    readonly id: string;
    private readonly config;
    private readonly fetchImplementation;
    private source;
    constructor(config?: HttpSseTransportConfig);
    connect(context: ConversationConnectContext, handlers: ConversationTransportHandlers): Promise<ConversationConnection>;
    send(request: SendMessageRequest): Promise<SendResult>;
    pause(request: {
        conversationId: string;
        signal?: AbortSignal;
    }): Promise<CommandResult>;
    close(request: {
        conversationId: string;
        signal?: AbortSignal;
    }): Promise<CommandResult>;
    resolveToolPermission(request: ResolveToolPermissionRequest): Promise<CommandResult>;
    disconnect(): void;
    private request;
    private url;
}
export declare function defaultEnvelopeEvents(envelope: RuntimeEventEnvelope): ConversationTransportEvent[];
//# sourceMappingURL=http-sse.d.ts.map