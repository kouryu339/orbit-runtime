export type RuntimeEventEnvelope<T = unknown> = {
  schema?: 'agent-runtime-event/v1' | string;
  type: string;
  conversation_id?: string;
  event_seq?: number;
  payload?: T;
};

export type LedgerRole =
  | 'user'
  | 'assistant'
  | 'system'
  | 'tool'
  | 'gateway_message'
  | 'agent_report'
  | 'summary';

export type LedgerRecord = {
  record_id?: string | number;
  id?: string;
  role?: LedgerRole;
  content?: string;
  text?: string;
  created_at?: string;
  metadata?: {
    subtype?: string;
    title?: string;
    tool_name?: string;
    tool_command?: string;
    call_id?: string;
    success?: boolean;
    display_content?: string;
    extra?: Record<string, unknown>;
    [key: string]: unknown;
  };
};

export type FrontendSnapshotPayload = {
  revision?: number;
  conversation_event_seq?: number;
  conversation_state?: ConversationRuntimeState;
  ledger?: LedgerRecord[];
  ledger_records?: LedgerRecord[];
  ledger_delta?: {
    kind?: 'append' | 'replace';
    record?: LedgerRecord;
    records?: LedgerRecord[];
  };
  agents?: Array<Record<string, unknown>>;
  plan?: unknown;
  error?: string;
  pending_permissions?: PendingToolPermission[];
  pendingPermissions?: PendingToolPermission[];
};

export type ToolEffect = 'read_only' | 'controlled_change' | 'destructive';

export type PendingToolPermission = {
  conversation_id: string;
  tool_call_id: string;
  agent_id: string;
  tool_name: string;
  display_name: string;
  effect: ToolEffect;
  arguments?: Record<string, unknown>;
  turn_id?: number;
  created_at?: string;
};

export type ConversationRuntimeState =
  | 'waiting'
  | 'thinking'
  | 'executing'
  | 'compacting'
  | 'stopping';

export type ToolCallStatus =
  | 'placeholder'
  | 'waiting_permission'
  | 'running'
  | 'finished'
  | 'failed';

export type ToolCallView = {
  id: string;
  title: string;
  status: ToolCallStatus;
  detail: string;
  toolName: string;
  command?: string;
};

export type PendingUserMessage = {
  id: string;
  content: string;
  createdAt: string;
  state: 'sending' | 'accepted' | 'failed';
  error?: string;
};

export type ConversationConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting';

export type ConversationState = {
  conversationId: string | null;
  connection: ConversationConnectionState;
  initialized: boolean;
  snapshotReceived: boolean;
  revision: number;
  eventSeq: number;
  runtimeState: ConversationRuntimeState;
  awaitingAssistantResponse: boolean;
  turnObservedRunning: boolean;
  turnObservedAssistant: boolean;
  records: LedgerRecord[];
  pendingUserMessages: PendingUserMessage[];
  toolCalls: ToolCallView[];
  agents: Array<Record<string, unknown>>;
  plan?: unknown;
  pendingPermissions: PendingToolPermission[];
  lastError?: string;
};

export type ConversationAction =
  | { type: 'connection'; state: ConversationConnectionState }
  | { type: 'conversation-created'; conversationId: string; eventSeq?: number }
  | { type: 'conversation-closed'; conversationId?: string }
  | { type: 'snapshot'; payload: FrontendSnapshotPayload; eventSeq?: number }
  | { type: 'local-message-added'; message: PendingUserMessage }
  | { type: 'local-message-accepted'; id: string }
  | { type: 'local-message-failed'; id: string; error: string }
  | { type: 'clear-error' }
  | { type: 'reset'; conversationId?: string | null };
