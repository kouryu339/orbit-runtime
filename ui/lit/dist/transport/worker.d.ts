import { type ConversationTransport, type ConversationConnectContext, type ConversationTransportHandlers, type SendMessageRequest, type ConversationCommandRequest, type ResolveToolPermissionRequest } from "../host/types.js";
/** Structural contract: both UI implementations can use @orbit-runtime/browser. */
export interface WorkerRuntimeClient {
    invoke<T = any>(type: string, payload?: any, options?: {
        commandId?: string;
    }): Promise<T>;
    subscribe(listener: (event: any) => void): () => void;
}
export declare class WorkerConversationTransport implements ConversationTransport {
    private readonly runtime;
    readonly contract: "agent-conversation-transport/v1";
    readonly id = "browser-worker";
    constructor(runtime: WorkerRuntimeClient);
    connect(context: ConversationConnectContext, handlers: ConversationTransportHandlers): Promise<{
        conversationId: string | undefined;
        disconnect: () => void;
    }>;
    send(request: SendMessageRequest): Promise<{
        accepted: boolean;
        commandId: any;
    }>;
    pause(request: ConversationCommandRequest): Promise<{
        accepted: boolean;
    }>;
    close(request: ConversationCommandRequest): Promise<{
        accepted: boolean;
    }>;
    requestSnapshot(request: ConversationCommandRequest): Promise<void>;
    resolveToolPermission(request: ResolveToolPermissionRequest): Promise<{
        accepted: boolean;
    }>;
}
//# sourceMappingURL=worker.d.ts.map