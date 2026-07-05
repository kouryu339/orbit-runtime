import { type CommandResult, type ConversationConnectContext, type ConversationConnection, type ConversationTransport, type ConversationTransportEvent, type ConversationTransportHandlers, type SendMessageRequest, type SendResult, type ResolveToolPermissionRequest } from '../host/types.js';
import type { RuntimeEventEnvelope } from '../protocol/types.js';
export type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
export type TauriListen = <T>(event: string, handler: (event: {
    payload: T;
}) => void) => Promise<() => void>;
export type TauriConversationTransportConfig = {
    id?: string;
    invoke: TauriInvoke;
    listen: TauriListen;
    commands?: {
        prepare?: string;
        send?: string;
        pause?: string;
        close?: string;
        snapshot?: string;
        pendingMessages?: string;
        resolveToolPermission?: string;
    };
    events?: {
        runtime?: string;
        pendingMessage?: string;
    };
    prepareArgs?: (context: ConversationConnectContext) => Record<string, unknown> | Promise<Record<string, unknown>>;
    sendArgs?: (request: SendMessageRequest) => Record<string, unknown> | Promise<Record<string, unknown>>;
    commandArgs?: (conversationId: string) => Record<string, unknown> | Promise<Record<string, unknown>>;
    snapshotArgs?: (conversationId: string | null) => Record<string, unknown> | Promise<Record<string, unknown>>;
    permissionArgs?: (request: ResolveToolPermissionRequest) => Record<string, unknown> | Promise<Record<string, unknown>>;
    mapEvent?: (envelope: RuntimeEventEnvelope) => ConversationTransportEvent[];
};
export declare class TauriConversationTransport implements ConversationTransport {
    readonly contract: "agent-conversation-transport/v1";
    readonly id: string;
    private readonly config;
    private readonly unlisten;
    private handlers;
    private conversationId;
    constructor(config: TauriConversationTransportConfig);
    connect(context: ConversationConnectContext, handlers: ConversationTransportHandlers): Promise<ConversationConnection>;
    send(request: SendMessageRequest): Promise<SendResult>;
    requestSnapshot(request: {
        conversationId: string;
    }): Promise<void>;
    pause(request: {
        conversationId: string;
    }): Promise<CommandResult>;
    close(request: {
        conversationId: string;
    }): Promise<CommandResult>;
    resolveToolPermission(request: ResolveToolPermissionRequest): Promise<CommandResult>;
    disconnect(): void;
    private loadSnapshot;
    private loadPendingMessages;
    private handleEnvelope;
    private handlePendingMessage;
    private commandArgs;
}
//# sourceMappingURL=tauri.d.ts.map