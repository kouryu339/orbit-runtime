import {
  EXTENSION_CONTRACT,
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
  LedgerRecord,
  RuntimeEventEnvelope,
} from '../protocol/types.js';

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
  mapContext?: (
    value: unknown,
    context: ConversationConnectContext,
  ) => ConversationTransportEvent[];
  mapEvent?: (
    envelope: RuntimeEventEnvelope,
  ) => ConversationTransportEvent[];
  createSendBody?: (request: SendMessageRequest) => unknown;
};

export class HttpSseConversationTransport implements ConversationTransport {
  readonly contract = TRANSPORT_CONTRACT;
  readonly id: string;

  private readonly config: HttpSseTransportConfig;
  private readonly fetchImplementation: typeof globalThis.fetch;
  private source: EventSource | null = null;

  constructor(config: HttpSseTransportConfig = {}) {
    this.config = config;
    this.id = config.id ?? 'http-sse';
    this.fetchImplementation = config.fetch ?? globalThis.fetch.bind(globalThis);
  }

  async connect(
    context: ConversationConnectContext,
    handlers: ConversationTransportHandlers,
  ): Promise<ConversationConnection> {
    this.disconnect();
    handlers.connection('connected');

    if (this.config.contextPath !== null) {
      const initial = await this.request(
        this.config.contextPath ?? '/api/context',
        { method: 'GET', signal: context.signal },
      );
      const events = this.config.mapContext?.(initial, context) ??
        defaultContextEvents(initial, context);
      for (const event of events) handlers.event(event);
    }

    const source = (this.config.eventSourceFactory ?? ((url) => new EventSource(url)))(
      this.url(this.config.eventsPath ?? '/events'),
    );
    this.source = source;
    source.onopen = () => handlers.connection('connected');
    source.onerror = () => handlers.connection('reconnecting');
    source.onmessage = (message) => {
      try {
        const envelope = JSON.parse(message.data) as RuntimeEventEnvelope;
        const events = this.config.mapEvent?.(envelope) ??
          defaultEnvelopeEvents(envelope);
        for (const event of events) handlers.event(event);
      } catch (error) {
        handlers.error({
          code: 'invalid-sse-event',
          message: error instanceof Error ? error.message : String(error),
          recoverable: true,
          cause: error,
        });
      }
    };

    return {
      conversationId: context.conversationId ?? undefined,
      disconnect: () => this.disconnect(),
    };
  }

  async send(request: SendMessageRequest): Promise<SendResult> {
    const value = await this.request(this.config.sendPath ?? '/api/chat', {
      method: 'POST',
      signal: request.signal,
      body: JSON.stringify(
        this.config.createSendBody?.(request) ?? {
          message: request.content,
          conversation_id: request.conversationId,
          client_message_id: request.clientMessageId,
          metadata: request.metadata,
        },
      ),
    });
    return commandResult(value);
  }

  async pause(request: { conversationId: string; signal?: AbortSignal }): Promise<CommandResult> {
    const value = await this.request(this.config.pausePath ?? '/api/pause', {
      method: 'POST',
      signal: request.signal,
      body: JSON.stringify({ conversation_id: request.conversationId }),
    });
    return commandResult(value);
  }

  async close(request: { conversationId: string; signal?: AbortSignal }): Promise<CommandResult> {
    if (!this.config.closePath) return { accepted: true };
    const value = await this.request(this.config.closePath, {
      method: 'POST',
      signal: request.signal,
      body: JSON.stringify({ conversation_id: request.conversationId }),
    });
    return commandResult(value);
  }

  async resolveToolPermission(
    request: ResolveToolPermissionRequest,
  ): Promise<CommandResult> {
    const value = await this.request(
      this.config.permissionPath ?? '/api/tool-permission',
      {
        method: 'POST',
        signal: request.signal,
        body: JSON.stringify({
          conversation_id: request.conversationId,
          tool_call_id: request.toolCallId,
          decision: request.decision,
        }),
      },
    );
    const result = commandResult(value);
    const resolved = asRecord(value)?.resolved;
    return resolved === false
      ? { ...result, accepted: false, rejectReason: 'Permission request is no longer pending.' }
      : result;
  }

  disconnect(): void {
    this.source?.close();
    this.source = null;
  }

  private async request(path: string, init: RequestInit): Promise<unknown> {
    const configuredHeaders = typeof this.config.headers === 'function'
      ? await this.config.headers()
      : this.config.headers;
    const response = await this.fetchImplementation(this.url(path), {
      ...init,
      headers: {
        'Content-Type': 'application/json',
        ...configuredHeaders,
        ...init.headers,
      },
    });
    const contentType = response.headers.get('content-type') ?? '';
    const value = contentType.includes('application/json')
      ? await response.json()
      : await response.text();
    if (!response.ok || hasError(value)) {
      throw new Error(errorMessage(value) ?? `Request failed: ${response.status}`);
    }
    return value;
  }

  private url(path: string): string {
    const base = this.config.baseUrl ?? globalThis.location?.href ?? 'http://localhost/';
    const url = new URL(path, base);
    for (const [key, value] of Object.entries(this.config.query ?? {})) {
      if (value !== undefined) url.searchParams.set(key, value);
    }
    return url.toString();
  }
}

export function defaultEnvelopeEvents(
  envelope: RuntimeEventEnvelope,
): ConversationTransportEvent[] {
  if (envelope.type === 'conversation:created') {
    const payload = asRecord(envelope.payload);
    const conversationId = stringValue(payload?.conversation_id) ??
      envelope.conversation_id;
    return conversationId
      ? [{
          type: 'conversation-created',
          conversationId,
          eventSeq: envelope.event_seq,
          metadata: payload ?? undefined,
        }]
      : [];
  }
  if (envelope.type === 'conversation:closed') {
    const payload = asRecord(envelope.payload);
    const conversationId = stringValue(payload?.conversation_id) ??
      envelope.conversation_id;
    return conversationId
      ? [{
          type: 'conversation-closed',
          conversationId,
          eventSeq: envelope.event_seq,
          reason: stringValue(payload?.reason),
        }]
      : [];
  }
  if (envelope.type === 'frontend:state_snapshot') {
    return [{
      type: 'state-snapshot',
      conversationId: envelope.conversation_id,
      eventSeq: envelope.event_seq,
      payload: (envelope.payload ?? {}) as FrontendSnapshotPayload,
    }];
  }
  if (envelope.type === 'conversation.ledger_delta') {
    const payload = asRecord(envelope.payload);
    const kind = stringValue(payload?.kind) ?? stringValue(payload?.op) ?? 'append';
    const record = payload?.record;
    const records = Array.isArray(payload?.records) ? payload.records : undefined;
    if (!record && !records) return [];
    return [{
      type: 'state-snapshot',
      conversationId: envelope.conversation_id,
      eventSeq: envelope.event_seq,
      payload: {
        revision: envelope.event_seq,
        conversation_event_seq: envelope.event_seq,
        ledger_delta: {
          kind: kind === 'replace' ? 'replace' : 'append',
          record: record && typeof record === 'object' ? record as LedgerRecord : undefined,
          records: records as LedgerRecord[] | undefined,
        },
      },
    }];
  }
  if (envelope.type === 'conversation.state_delta') {
    const events: ConversationTransportEvent[] = [];
    const payload = stateSnapshotPayloadFromStateDelta(envelope);
    if (payload) {
      events.push({
        type: 'state-snapshot',
        conversationId: envelope.conversation_id,
        eventSeq: envelope.event_seq,
        payload,
      });
    }
    events.push(runtimeExtensionEvent(envelope, 'org.agent-runtime.state-delta'));
    return events;
  }
  return [runtimeExtensionEvent(envelope, 'org.agent-runtime.event')];
}

function stateSnapshotPayloadFromStateDelta(
  envelope: RuntimeEventEnvelope,
): FrontendSnapshotPayload | null {
  const delta = asRecord(envelope.payload);
  if (!delta) return null;
  const payload: FrontendSnapshotPayload = {
    revision: envelope.event_seq,
    conversation_event_seq: envelope.event_seq,
  };
  const state = stringValue(delta.conversation_state) ?? stringValue(delta.state);
  if (state) payload.conversation_state = state as FrontendSnapshotPayload['conversation_state'];
  if (Array.isArray(delta.agents)) payload.agents = delta.agents as Array<Record<string, unknown>>;
  if (delta.plan !== undefined) payload.plan = delta.plan;
  const pendingPermissions = delta.pending_permissions ?? delta.pendingPermissions;
  if (Array.isArray(pendingPermissions)) {
    payload.pending_permissions = pendingPermissions as FrontendSnapshotPayload['pending_permissions'];
  }
  return payload.conversation_state ||
    payload.agents ||
    payload.plan !== undefined ||
    payload.pending_permissions
    ? payload
    : null;
}

function runtimeExtensionEvent(
  envelope: RuntimeEventEnvelope,
  namespace: string,
): ConversationTransportEvent {
  return {
    type: 'transport-extension',
    extension: {
      contract: EXTENSION_CONTRACT,
      namespace,
      kind: envelope.type,
      version: 1,
      payload: envelope.payload,
    },
  };
}

function defaultContextEvents(
  value: unknown,
  context: ConversationConnectContext,
): ConversationTransportEvent[] {
  const record = asRecord(value);
  const conversationId =
    stringValue(record?.conversation_id) ??
    stringValue(record?.supervisor_conversation_id) ??
    stringValue(record?.editor_conversation_id) ??
    context.conversationId ??
    undefined;
  const events: ConversationTransportEvent[] = [];
  if (conversationId) {
    events.push({ type: 'conversation-created', conversationId });
  }
  const snapshot =
    record?.snapshot ??
    record?.supervisor_snapshot ??
    record?.editor_snapshot;
  if (snapshot && typeof snapshot === 'object') {
    events.push({
      type: 'state-snapshot',
      conversationId,
      payload: snapshot as FrontendSnapshotPayload,
    });
  }
  events.push({
    type: 'transport-extension',
    extension: {
      contract: EXTENSION_CONTRACT,
      namespace: 'org.agent-runtime.http-context',
      kind: 'context',
      version: 1,
      payload: value,
    },
  });
  return events;
}

function commandResult(value: unknown): SendResult {
  const record = asRecord(value);
  const decision = asRecord(record?.decision);
  const rejected =
    record?.accepted === false ||
    decision?.decision === 'rejected' ||
    typeof record?.reject_reason === 'string';
  return {
    accepted: !rejected,
    commandId: stringValue(record?.command_id),
    rejectReason:
      stringValue(record?.reject_reason) ??
      stringValue(decision?.reason),
    metadata: record ?? undefined,
  };
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object'
    ? value as Record<string, unknown>
    : null;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' && value ? value : undefined;
}

function hasError(value: unknown): boolean {
  return Boolean(asRecord(value)?.error);
}

function errorMessage(value: unknown): string | undefined {
  const record = asRecord(value);
  return stringValue(record?.error) ?? stringValue(record?.message);
}
