import type {
  ConversationConnectionState,
  ConversationState,
  FrontendSnapshotPayload,
} from '../protocol/types.js';

export const HOST_CONTRACT = 'agent-conversation-host/v1' as const;
export const TRANSPORT_CONTRACT = 'agent-conversation-transport/v1' as const;
export const PRESENTATION_CONTRACT =
  'agent-conversation-presentation/v1' as const;
export const EXTENSION_CONTRACT = 'agent-conversation-extension/v1' as const;

export type ExtensibleString<T extends string> = T | (string & {});

export type ExtensionEnvelope<T = unknown> = {
  contract: typeof EXTENSION_CONTRACT;
  namespace: string;
  kind: string;
  version: number;
  payload: T;
};

export type ConversationConnectContext = {
  conversationId?: string | null;
  locale?: string;
  metadata?: Record<string, unknown>;
  signal?: AbortSignal;
};

export type ConversationTransportEvent =
  | {
      type: 'conversation-created';
      conversationId: string;
      eventSeq?: number;
      metadata?: Record<string, unknown>;
    }
  | {
      type: 'conversation-closed';
      conversationId: string;
      eventSeq?: number;
      reason?: string;
    }
  | {
      type: 'state-snapshot';
      conversationId?: string;
      eventSeq?: number;
      payload: FrontendSnapshotPayload;
    }
  | {
      type: 'pending-user-message';
      conversationId: string;
      messageId: string;
      content: string;
      createdAt?: string;
    }
  | {
      type: 'transport-extension';
      extension: ExtensionEnvelope;
    };

export type ConversationTransportError = {
  code: string;
  message: string;
  recoverable: boolean;
  cause?: unknown;
  metadata?: Record<string, unknown>;
};

export type ConversationTransportHandlers = {
  event(event: ConversationTransportEvent): void;
  connection(
    state: Exclude<ConversationConnectionState, 'connecting'>,
  ): void;
  error(error: ConversationTransportError): void;
};

export type ConversationConnection = {
  readonly conversationId?: string;
  disconnect(): void | Promise<void>;
};

export type ConversationDescriptor = {
  conversationId: string;
  title?: string;
  clusterId?: string;
  createdAt?: string;
  updatedAt?: string;
  preview?: string;
  metadata?: Record<string, unknown>;
};

export type ConversationListQuery = {
  cursor?: string;
  limit?: number;
  clusterId?: string;
  search?: string;
};

export type ConversationPage = {
  items: ConversationDescriptor[];
  nextCursor?: string;
};

export type ConversationCreateRequest = {
  title?: string;
  clusterId?: string;
  metadata?: Record<string, unknown>;
};

export interface ConversationRepository {
  list(query?: ConversationListQuery): Promise<ConversationPage>;
  create(request?: ConversationCreateRequest): Promise<ConversationDescriptor>;
  open(conversationId: string): Promise<ConversationDescriptor>;
  rename?(conversationId: string, title: string): Promise<void>;
  delete?(conversationId: string): Promise<void>;
}

export type ConversationCommandRequest = {
  conversationId: string;
  signal?: AbortSignal;
  metadata?: Record<string, unknown>;
};

export type SendMessageRequest = ConversationCommandRequest & {
  content: string;
  clientMessageId: string;
};

export type SendResult = {
  accepted: boolean;
  commandId?: string;
  rejectReason?: string;
  metadata?: Record<string, unknown>;
};

export type CommandResult = {
  accepted: boolean;
  commandId?: string;
  rejectReason?: string;
  metadata?: Record<string, unknown>;
};

export type ResolveToolPermissionRequest = ConversationCommandRequest & {
  toolCallId: string;
  decision: 'allow' | 'deny';
};

export interface ConversationTransport {
  readonly contract: typeof TRANSPORT_CONTRACT;
  readonly id: string;
  readonly repository?: ConversationRepository;
  connect(
    context: ConversationConnectContext,
    handlers: ConversationTransportHandlers,
  ): Promise<ConversationConnection>;
  send(request: SendMessageRequest): Promise<SendResult>;
  pause?(request: ConversationCommandRequest): Promise<CommandResult>;
  close?(request: ConversationCommandRequest): Promise<CommandResult>;
  requestSnapshot?(request: ConversationCommandRequest): Promise<void>;
  resolveToolPermission?(
    request: ResolveToolPermissionRequest,
  ): Promise<CommandResult>;
}

export type OpenLinkRequest = {
  url: string;
  source?: string;
};

export type OpenImageRequest = {
  source: string;
  alt?: string;
  baseDir?: string;
  metadata?: Record<string, unknown>;
};

export type PickPathRequest = {
  mode: 'file' | 'directory';
  label?: string;
  accept?: string[];
  multiple?: boolean;
};

export type PickPathResult = {
  paths: string[];
};

export type DownloadRequest = {
  source: string;
  suggestedName?: string;
  metadata?: Record<string, unknown>;
};

export type ConversationHostCapabilities = {
  openLink?(request: OpenLinkRequest): Promise<void>;
  openImage?(request: OpenImageRequest): Promise<void>;
  pickPath?(request: PickPathRequest): Promise<PickPathResult | null>;
  copyText?(text: string): Promise<void>;
  download?(request: DownloadRequest): Promise<void>;
};

export type ProviderDefinition = {
  uid: number;
  name?: string | null;
  type?: string;
  builtin_type?: string;
  builtinType?: string;
  base_url?: string;
  baseUrl?: string;
  api_paradigm?: string | null;
  apiParadigm?: string | null;
  prompt_cache_control?: boolean;
  promptCacheControl?: boolean;
  api_key_set?: boolean;
  apiKeySet?: boolean;
};

export type ProviderModelDefinition = {
  uid: number;
  provider_uid?: number;
  providerUid?: number;
  model_name?: string;
  modelName?: string;
  model_id?: string;
  modelId?: string;
  name?: string;
  context_window?: number;
  contextWindow?: number;
};

export type ProviderDefinitionsResult = {
  schema?: string;
  providers?: ProviderDefinition[];
  models?: ProviderModelDefinition[];
  current_model_uid?: number | null;
  currentModelUid?: number | null;
};

export type BuiltinProviderCatalogProvider = {
  id: string;
  name: string;
  prefix?: string;
  defaultBaseUrl?: string;
  default_base_url?: string;
  apiFormat?: string;
  api_format?: string;
};

export type BuiltinProviderCatalogModel = {
  id: string;
  name: string;
  developer?: string;
  contextWindow?: number;
  context_window?: number;
  providerPrefix?: string;
  provider_prefix?: string;
  default?: boolean;
};

export type BuiltinProviderCatalog = {
  providers?: BuiltinProviderCatalogProvider[];
  models?: BuiltinProviderCatalogModel[];
};

export type ConfigureProvidersRequest = {
  input: string;
  source?: 'path' | 'json' | 'text';
  metadata?: Record<string, unknown>;
};

export type SetCurrentModelRequest = {
  modelUid: number;
  metadata?: Record<string, unknown>;
};

export interface ConversationProviderController {
  getProviderDefinitions(): Promise<ProviderDefinitionsResult>;
  getBuiltinProviderCatalog?(): Promise<BuiltinProviderCatalog>;
  configureProviders(request: ConfigureProvidersRequest): Promise<CommandResult>;
  setCurrentModel(request: SetCurrentModelRequest): Promise<CommandResult>;
}

export type ConversationProviderControlsPolicy =
  | { enabled: false }
  | {
      enabled: true;
      controller: ConversationProviderController;
      showModelSwitcher?: boolean;
      showImport?: boolean;
    };

export type PresentationAnchor =
  | { type: 'head' }
  | { type: 'tail' }
  | { type: 'before-record'; recordId: string }
  | { type: 'after-record'; recordId: string };

export type ConversationPresentationItem = {
  contract: typeof PRESENTATION_CONTRACT;
  id: string;
  scope?: string;
  kind: ExtensibleString<'assistant-markdown' | 'notice'>;
  anchor: PresentationAnchor;
  content?: string;
  reveal?: 'none' | 'progressive';
  createdAt?: string;
  metadata?: Record<string, unknown>;
  extension?: ExtensionEnvelope;
};

export type PresentationItemPatch = Partial<
  Omit<ConversationPresentationItem, 'contract' | 'id'>
>;

export type PresetMarkdown = {
  id?: string;
  name?: string;
  markdown: string;
  baseDir?: string;
  createdAt?: string;
  scope?: string;
};

export type ConversationColorScheme = 'light' | 'dark' | 'system';
export type ConversationThemeName = ExtensibleString<
  | 'studio'
  | 'paper'
  | 'soft'
  | 'midnight'
  | 'neutral'
  | 'blue'
  | 'green'
  | 'amber'
>;
export type ConversationDensity = 'compact' | 'comfortable';

export const PERSISTENCE_CONTRACT =
  'agent-conversation-persistence/v1' as const;

export type PersistedConversationStatus =
  | 'sealed'
  | 'running'
  | 'saving'
  | 'restoring'
  | 'error';

export type ConversationInstanceState =
  | 'current'
  | 'background'
  | 'data_only';

export type PersistedConversation = {
  archiveId: string;
  runtimeConversationId: string | null;
  title?: string;
  preview?: string;
  clusterId?: string;
  createdAt?: string;
  updatedAt?: string;
  status: PersistedConversationStatus;
  instanceState?: ConversationInstanceState;
  metadata?: Record<string, unknown>;
};

export type PersistedConversationPage = {
  items: PersistedConversation[];
  nextCursor?: string;
};

export type ConversationBinding = {
  archiveId?: string;
  runtimeConversationId: string;
};

export type ConversationPersistenceEvent =
  | { type: 'archive-updated'; archive: PersistedConversation }
  | { type: 'archive-deleted'; archiveId: string }
  | { type: 'binding-changed'; binding: ConversationBinding | null }
  | {
      type: 'operation-state';
      operation: 'list' | 'save' | 'restore' | 'create' | 'close';
      pending: boolean;
      archiveId?: string;
    }
  | { type: 'error'; operation: string; message: string; archiveId?: string };

export interface ConversationPersistenceController {
  readonly contract: typeof PERSISTENCE_CONTRACT;
  list(query?: {
    cursor?: string;
    limit?: number;
    search?: string;
  }): Promise<PersistedConversationPage>;
  create?(request?: {
    title?: string;
    clusterId?: string;
    metadata?: Record<string, unknown>;
  }): Promise<ConversationBinding>;
  save(request: {
    archiveId?: string;
    runtimeConversationId: string;
    title?: string;
    metadata?: Record<string, unknown>;
  }): Promise<PersistedConversation>;
  restore(request: {
    archiveId: string;
    signal?: AbortSignal;
  }): Promise<ConversationBinding>;
  close?(request: {
    archiveId?: string;
    runtimeConversationId: string;
    save?: boolean;
  }): Promise<void>;
  rename?(archiveId: string, title: string): Promise<void>;
  delete?(archiveId: string): Promise<void>;
  subscribe?(
    listener: (event: ConversationPersistenceEvent) => void,
  ): () => void;
}

export type ConversationPersistencePolicy =
  | { enabled: false }
  | {
      enabled: true;
      controller: ConversationPersistenceController;
      binding?: ConversationBinding | null;
      showHistory?: boolean;
    };

export type SendOptions = {
  source?: string;
  metadata?: Record<string, unknown>;
  signal?: AbortSignal;
};

export type ConversationExtension = {
  namespace: string;
  kinds: string[];
  version: number;
};

export interface AgentRuntimeConversationPublicApi {
  transport: ConversationTransport | null;
  capabilities: ConversationHostCapabilities;
  presentationItems: readonly ConversationPresentationItem[];
  extensions: readonly ConversationExtension[];
  conversationId: string | null;
  locale: string;
  colorScheme: ConversationColorScheme;
  theme: ConversationThemeName;
  density: ConversationDensity;
  persistence: ConversationPersistencePolicy;
  providerControls: ConversationProviderControlsPolicy;
  readonly state: ConversationState;
  connect(): Promise<void>;
  disconnect(): void;
  openConversation(conversationId: string): Promise<void>;
  createConversation(): Promise<void>;
  send(content: string, options?: SendOptions): Promise<SendResult>;
  pause(): Promise<CommandResult>;
  resolveToolPermission(
    toolCallId: string,
    decision: 'allow' | 'deny',
  ): Promise<CommandResult>;
  closeConversation(): Promise<void>;
  saveConversation(): Promise<PersistedConversation | null>;
  restoreConversation(archiveId: string): Promise<void>;
  refreshConversationHistory(): Promise<void>;
  focusComposer(): void;
  insertPresetMarkdown(preset: PresetMarkdown): string;
  insertPresentationItem(item: ConversationPresentationItem): void;
  updatePresentationItem(id: string, patch: PresentationItemPatch): void;
  removePresentationItem(id: string): void;
  clearPresentationItems(scope?: string): void;
}

export type ConversationPublicEventMap = {
  'agent-conversation-ready': {
    conversationId: string;
    state: ConversationState;
  };
  'agent-conversation-state-change': {
    state: ConversationState;
    reason: string;
  };
  'agent-conversation-send': {
    conversationId: string;
    clientMessageId: string;
    content: string;
    source?: string;
    result: SendResult;
  };
  'agent-conversation-pause': {
    conversationId: string;
    result: CommandResult;
  };
  'agent-conversation-tool-permission': {
    conversationId: string;
    toolCallId: string;
    decision: 'allow' | 'deny';
    result: CommandResult;
  };
  'agent-conversation-connection-change': {
    transportId: string;
    state: ConversationConnectionState;
  };
  'agent-conversation-presentation-action': {
    item: ConversationPresentationItem;
    action: string;
    value?: unknown;
  };
  'agent-conversation-provider-action': {
    action: string;
    modelUid?: number;
    source?: 'path' | 'json' | 'text';
    result?: CommandResult;
    definitions?: ProviderDefinitionsResult;
  };
  'agent-conversation-extension-action': {
    extension: ExtensionEnvelope;
    action: string;
    value?: unknown;
  };
  'agent-conversation-error': ConversationTransportError;
  'agent-conversation-diagnostic': {
    code: string;
    message: string;
    metadata?: Record<string, unknown>;
  };
};
