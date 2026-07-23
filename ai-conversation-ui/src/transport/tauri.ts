import {
  TRANSPORT_CONTRACT,
  type CommandResult,
  type ConversationConnectContext,
  type ConversationConnection,
  type ConversationTransport,
  type ConversationTransportEvent,
  type ConversationTransportHandlers,
  type SendMessageRequest,
  type SendResult,
  type ResolveToolPermissionRequest,
} from '../host/types.js';
import type {
  FrontendSnapshotPayload,
  RuntimeEventEnvelope,
} from '../protocol/types.js';
import { defaultEnvelopeEvents } from './http-sse.js';

export type TauriInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export type TauriListen = <T>(
  event: string,
  handler: (event: { payload: T }) => void,
) => Promise<() => void>;

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
  prepareArgs?: (
    context: ConversationConnectContext,
  ) => Record<string, unknown> | Promise<Record<string, unknown>>;
  sendArgs?: (
    request: SendMessageRequest,
  ) => Record<string, unknown> | Promise<Record<string, unknown>>;
  commandArgs?: (
    conversationId: string,
  ) => Record<string, unknown> | Promise<Record<string, unknown>>;
  snapshotArgs?: (
    conversationId: string | null,
  ) => Record<string, unknown> | Promise<Record<string, unknown>>;
  permissionArgs?: (
    request: ResolveToolPermissionRequest,
  ) => Record<string, unknown> | Promise<Record<string, unknown>>;
  mapEvent?: (
    envelope: RuntimeEventEnvelope,
  ) => ConversationTransportEvent[];
};

type PreparedConversation = {
  conversation_id?: string;
  conversationId?: string;
};

type PendingMessage = {
  message_id?: string;
  messageId?: string;
  conversation_id?: string;
  conversationId?: string;
  content?: string;
  created_at?: string;
  createdAt?: string;
};

export class TauriConversationTransport implements ConversationTransport {
  readonly contract = TRANSPORT_CONTRACT;
  readonly id: string;

  private readonly config: TauriConversationTransportConfig;
  private readonly unlisten = new Set<() => void>();
  private handlers: ConversationTransportHandlers | null = null;
  private conversationId: string | null = null;

  constructor(config: TauriConversationTransportConfig) {
    this.config = config;
    this.id = config.id ?? 'tauri-events';
  }

  async connect(
    context: ConversationConnectContext,
    handlers: ConversationTransportHandlers,
  ): Promise<ConversationConnection> {
    this.disconnect();
    this.handlers = handlers;
    handlers.connection('connected');

    const runtimeEvent = this.config.events?.runtime ?? 'ai-frontend-event';
    const unlistenRuntime = await this.config.listen<RuntimeEventEnvelope>(
      runtimeEvent,
      (event) => this.handleEnvelope(event.payload),
    );
    this.unlisten.add(unlistenRuntime);

    const pendingEvent =
      this.config.events?.pendingMessage ?? 'ai-pending-user-message';
    const unlistenPending = await this.config.listen<PendingMessage>(
      pendingEvent,
      (event) => this.handlePendingMessage(event.payload),
    );
    this.unlisten.add(unlistenPending);

    this.conversationId = context.conversationId ?? null;
    if (!this.conversationId) {
      const prepare = this.config.commands?.prepare ?? 'ai_prepare_conversation';
      const args = await this.config.prepareArgs?.(context) ?? {};
      const created = await this.config.invoke<PreparedConversation>(prepare, args);
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

  async send(request: SendMessageRequest): Promise<SendResult> {
    const command = this.config.commands?.send ?? 'ai_send_message';
    const args = await this.config.sendArgs?.(request) ?? {
      args: {
        conversationId: request.conversationId,
        content: request.content,
        clientMessageId: request.clientMessageId,
        metadata: request.metadata,
      },
    };
    const value = await this.config.invoke<unknown>(command, args);
    const result = normalizeCommandResult(value);
    return result;
  }

  async requestSnapshot(request: { conversationId: string }): Promise<void> {
    if (!this.handlers || request.conversationId !== this.conversationId) return;
    await this.loadSnapshot();
  }

  async pause(request: { conversationId: string }): Promise<CommandResult> {
    const command = this.config.commands?.pause ?? 'ai_pause_conversation';
    const args = await this.commandArgs(request.conversationId);
    return normalizeCommandResult(
      await this.config.invoke<unknown>(command, args),
    );
  }

  async close(request: { conversationId: string }): Promise<CommandResult> {
    const command = this.config.commands?.close ?? 'ai_close_conversation';
    const args = await this.commandArgs(request.conversationId);
    const result = normalizeCommandResult(
      await this.config.invoke<unknown>(command, args),
    );
    if (request.conversationId === this.conversationId) {
      this.conversationId = null;
    }
    return result;
  }

  async resolveToolPermission(
    request: ResolveToolPermissionRequest,
  ): Promise<CommandResult> {
    const command = this.config.commands?.resolveToolPermission ??
      'ai_resolve_tool_permission';
    const args = await this.config.permissionArgs?.(request) ?? {
      args: {
        conversation_id: request.conversationId,
        tool_call_id: request.toolCallId,
        decision: request.decision,
      },
    };
    return normalizeCommandResult(
      await this.config.invoke<unknown>(command, args),
    );
  }

  disconnect(): void {
    for (const unlisten of this.unlisten) unlisten();
    this.unlisten.clear();
    this.handlers = null;
  }

  private async loadSnapshot(): Promise<void> {
    const command = this.config.commands?.snapshot ?? 'ai_runtime_snapshot';
    const args = await this.config.snapshotArgs?.(this.conversationId) ??
      (this.conversationId ? { args: { conversationId: this.conversationId } } : {});
    const value = await this.config.invoke<unknown>(command, args);
    if (!value || typeof value !== 'object') return;
    const envelope = value as RuntimeEventEnvelope;
    if (typeof envelope.type === 'string') {
      this.handleEnvelope(envelope);
      return;
    }
    this.handlers?.event({
      type: 'state-snapshot',
      conversationId: this.conversationId ?? undefined,
      payload: value as FrontendSnapshotPayload,
    });
  }

  private async loadPendingMessages(): Promise<void> {
    const command =
      this.config.commands?.pendingMessages ?? 'ai_pending_messages';
    const messages = await this.config.invoke<PendingMessage[]>(command);
    for (const message of messages ?? []) this.handlePendingMessage(message);
  }

  private handleEnvelope(envelope: RuntimeEventEnvelope): void {
    const events = this.config.mapEvent?.(envelope) ??
      defaultEnvelopeEvents(envelope);
    for (const event of events) {
      if (
        event.type === 'conversation-created' &&
        !this.conversationId
      ) {
        this.conversationId = event.conversationId;
      }
      this.handlers?.event(event);
    }
  }

  private handlePendingMessage(message: PendingMessage): void {
    const conversationId =
      message.conversation_id ?? message.conversationId;
    const messageId = message.message_id ?? message.messageId;
    if (!conversationId || !messageId || !message.content) return;
    if (this.conversationId && conversationId !== this.conversationId) return;
    this.handlers?.event({
      type: 'pending-user-message',
      conversationId,
      messageId,
      content: message.content,
      createdAt: message.created_at ?? message.createdAt,
    });
  }

  private async commandArgs(
    conversationId: string,
  ): Promise<Record<string, unknown>> {
    return await this.config.commandArgs?.(conversationId) ?? {
      args: { conversationId },
    };
  }
}

function normalizeCommandResult(value: unknown): CommandResult {
  if (!value || typeof value !== 'object') return { accepted: true };
  const record = value as Record<string, unknown>;
  const accepted = record.accepted !== false && record.resolved !== false;
  return {
    accepted,
    commandId:
      typeof record.command_id === 'string'
        ? record.command_id
        : typeof record.commandId === 'string'
          ? record.commandId
          : record.resolved === false
            ? 'Permission request is no longer pending.'
            : undefined,
    rejectReason:
      typeof record.reject_reason === 'string'
        ? record.reject_reason
        : typeof record.rejectReason === 'string'
          ? record.rejectReason
          : undefined,
    metadata: record,
  };
}
