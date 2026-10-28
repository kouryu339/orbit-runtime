<svelte:options accessors />

<script lang="ts">
  import { createEventDispatcher, onMount, tick } from 'svelte';
  import {
    createDisplayItems,
    latestWidgetRecordKey,
    latestAssistantKey,
    ComposerSession,
    effortLabel,
  } from '@agent-runtime/conversation-element/content';
  import {
    conversationReducer,
    createConversationState,
    createPendingUserMessage,
    displayText,
    displayTextWithToolAnchors,
    recordKey,
    summarizePermissionTargets,
    snapshotRecords,
    type ConversationAction,
    type ConversationState,
    type LedgerRecord,
  } from '@agent-runtime/conversation-element/protocol';
  import type {
    ConversationPersistenceController,
    ConversationProviderController,
    ConversationTransport,
    ConversationTransportError,
    ConversationTransportEvent,
    PersistedConversation,
    ProviderDefinitionsResult,
    ProviderModelDefinition,
    SendResult,
    SendOptions,
    CommandResult,
    ConversationHostCapabilities,
    ConversationExtension,
    ConversationPresentationItem,
    PresentationItemPatch,
    PresetMarkdown,
    ConversationPersistencePolicy,
    ConversationProviderControlsPolicy,
    ConversationThemeName,
    ConversationDensity,
    ConversationPublicEventMap,
    ConversationComposerController,
  } from '@agent-runtime/conversation-element/host';
  import RichContent from './RichContent.svelte';
  import ProviderEditor from './ProviderEditor.svelte';
  import PlanCard from './PlanCard.svelte';
  import UserRecord from './UserRecord.svelte';
  import './conversation.css';

  export let transport: ConversationTransport | null = null;
  export let capabilities: ConversationHostCapabilities = {};
  export let extensions: readonly ConversationExtension[] = [];
  export let presentationItems: readonly ConversationPresentationItem[] = [];
  export let persistence: ConversationPersistencePolicy = { enabled: false };
  export let providerControls: ConversationProviderControlsPolicy = {
    enabled: false,
  };
  export let composerController: ConversationComposerController | null = null;
  export let theme: ConversationThemeName = 'studio';
  export let density: ConversationDensity = 'comfortable';
  export let conversationId: string | null = null;
  export let locale = 'en-US';
  export let progressiveReveal = true;
  export let colorScheme: 'light' | 'dark' | 'system' = 'system';
  $: persistenceController = persistence.enabled
    ? persistence.controller
    : null;
  $: providerController = providerControls.enabled
    ? providerControls.controller
    : null;
  export let hideToolCalls = false;
  export let disclaimer = '';

  export let state: ConversationState = createConversationState(conversationId);
  const emit = createEventDispatcher<ConversationPublicEventMap>();
  let generation = 0;
  let pausing = false;
  type PendingImage = { file: File; previewUrl: string; number: number };
  type MessagePart =
    | { type: 'text'; text: string }
    | { type: 'image'; image_id: string };
  let pendingImages: PendingImage[] = [];
  let imageSending = false;
  let imageInput: HTMLInputElement;
  let stickToBottom = true;
  let connectAbort: AbortController | undefined;
  let permissionOperation = '';
  let permissionErrorId = '';
  let mediaDark = false;
  const revealLengths = new Map<string, number>();
  let boundPersistence: ConversationPersistenceController | null = null;
  let boundProvider: ConversationProviderController | null = null;
  let composerElement: HTMLTextAreaElement;
  let completedReveal = new Set<string>();
  let unsubscribePersistence: (() => void) | undefined;
  let deferred: {
    content: string;
    options: SendOptions;
    resolve: (r: SendResult) => void;
    cleanup: () => void;
  } | null = null;
  let providerError = '';
  let editorOpen = false;
  let editorProviderUid: number | null = null;
  let connection: { disconnect(): void | Promise<void> } | null = null;
  let draft = '';
  let messagesElement: HTMLElement;
  let operation = '';
  let permissionError = '';
  let historyOpen = false;
  let providerOpen = false;
  let archives: PersistedConversation[] = [];
  let providers: ProviderDefinitionsResult | null = null;
  let composerVersion = 0;
  let composerPanel: 'model' | 'permissions' | null = null;
  let composerPanelPosition = '';
  const composerSession = new ComposerSession(
    () => (composerVersion += 1),
    (message) =>
      emit('agent-conversation-error', {
        code: 'composer-action-failed',
        message,
        recoverable: true,
      }),
  );
  $: composerSession.sync(
    state,
    transport,
    composerController,
    state.modelUid,
    operation === 'model',
  );
  function composerView(
    _state: ConversationState,
    _version: number,
    _operation: string,
  ) {
    return {
      phase: composerSession.phase,
      canCompose: composerSession.canCompose,
      canPause: composerSession.canPause,
      canResume: composerSession.canResume,
      canConfigure: composerSession.canConfigure,
      settings: composerSession.settings,
      displayedEffort: composerSession.displayedEffort,
      awaiting: composerSession.awaiting,
    };
  }
  $: composer = composerView(state, composerVersion, operation);
  $: composerPhase = imageSending ? 'sending' : composer.phase;
  $: composerBusy = [
    'thinking',
    'executing',
    'approval',
    'pausing',
    'resuming',
    'compacting',
    'sending',
  ].includes(composerPhase);
  $: composerResume =
    composerPhase === 'paused' && !draft.trim() && !pendingImages.length;
  $: composerActionLabel = composerBusy
    ? 'Pause response'
    : composerResume
      ? 'Continue response'
      : 'Send message';
  $: composerActionDisabled =
    Boolean(permissionOperation) ||
    (composerBusy
      ? !composer.canPause || imageSending
      : composerResume
        ? !composer.canResume
        : !canCompose || (!draft.trim() && !pendingImages.length));
  $: composerModel = providers?.models?.find((m) => m.uid === state.modelUid);
  $: composerEfforts = composer.settings.supportedEfforts ?? [];
  $: composerPermission = composer.settings.permissionModes?.find(
    (mode) => mode.id === composer.settings.permissionMode,
  );
  const composerStateLabels: Record<string, [string, string]> = {
    offline: ['未连接', 'Offline'],
    initializing: ['正在连接会话', 'Initializing'],
    sending: ['正在发送', 'Sending'],
    thinking: ['正在思考', 'Thinking'],
    executing: ['正在执行', 'Executing'],
    approval: ['等待批准', 'Waiting for approval'],
    pausing: ['正在暂停', 'Pausing'],
    resuming: ['正在继续', 'Resuming'],
    compacting: ['正在整理上下文', 'Compacting'],
    paused: ['已暂停', 'Paused'],
  };
  function toggleComposerPanel(
    panel: 'model' | 'permissions',
    event: MouseEvent,
  ) {
    composerSession.clearEffortPreview();
    if (composerPanel === panel) {
      composerPanel = null;
      return;
    }
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    composerPanelPosition = `left:${Math.max(12, Math.min(rect.right - 300, window.innerWidth - 312))}px;bottom:${Math.max(12, window.innerHeight - rect.top + 8)}px;max-height:${Math.max(120, rect.top - 24)}px`;
    composerPanel = panel;
  }
  let localSequence = 0;
  $: dark = colorScheme === 'dark' || (colorScheme === 'system' && mediaDark);
  $: displayItems = createDisplayItems(
    state,
    presentationItems,
    completedReveal,
  );
  $: latestWidget = latestWidgetRecordKey(displayItems);
  $: latestAssistant = latestAssistantKey(displayItems);
  $: zh = locale.toLowerCase().startsWith('zh');
  $: trackReveal(state, presentationItems);
  $: bindProviders(providerController);
  $: active =
    state.runtimeState !== 'waiting' || state.awaitingAssistantResponse;
  $: canCompose = composer.canCompose && !pausing && operation !== 'model';
  $: bindPersistence(persistenceController);
  $: if (conversationId !== null && conversationId !== state.conversationId) {
    dispatch({ type: 'reset', conversationId });
  }

  function bindPersistence(
    controller: ConversationPersistenceController | null,
  ) {
    if (controller === boundPersistence) return;
    unsubscribePersistence?.();
    boundPersistence = controller;
    archives = [];
    unsubscribePersistence = controller?.subscribe?.((event) => {
      if (event.type === 'archive-updated')
        archives = [
          event.archive,
          ...archives.filter((a) => a.archiveId !== event.archive.archiveId),
        ];
      else if (event.type === 'archive-deleted')
        archives = archives.filter((a) => a.archiveId !== event.archiveId);
      else if (event.type === 'binding-changed' && persistence.enabled) {
        persistence = { ...persistence, binding: event.binding };
      } else if (event.type === 'error')
        emit('agent-conversation-error', {
          code: `persistence-${event.operation}-failed`,
          message: event.message,
          recoverable: true,
          metadata: { archiveId: event.archiveId },
        });
    });
    if (controller && persistence.enabled && persistence.showHistory !== false)
      void refreshConversationHistory();
  }
  function bindProviders(controller: ConversationProviderController | null) {
    if (controller === boundProvider) return;
    boundProvider = controller;
    providers = null;
    providerOpen = false;
    editorOpen = false;
    if (controller) void refreshProviders();
  }
  function followReveal() {
    if (!stickToBottom) return;
    void tick().then(() => {
      if (messagesElement)
        messagesElement.scrollTop = messagesElement.scrollHeight;
    });
  }
  function completeReveal(key: string) {
    if (!completedReveal.has(key))
      completedReveal = new Set([...completedReveal, key]);
    followReveal();
  }
  function trackReveal(
    value: ConversationState,
    items: readonly ConversationPresentationItem[],
  ) {
    const current = new Map<string, number>();
    for (const record of value.records)
      if (record.role === 'assistant')
        current.set(recordKey(record), displayText(record).length);
    for (const item of items)
      current.set(`presentation:${item.id}`, item.content?.length ?? 0);
    const next = new Set(completedReveal);
    let changed = false;
    for (const [key, length] of current) {
      if (revealLengths.has(key) && length > revealLengths.get(key)!)
        changed = next.delete(key) || changed;
      revealLengths.set(key, length);
    }
    for (const key of revealLengths.keys())
      if (!current.has(key)) {
        revealLengths.delete(key);
        changed = next.delete(key) || changed;
      }
    if (changed) completedReveal = next;
  }
  function report(operation: string, cause: unknown, kind = 'persistence') {
    emit('agent-conversation-error', {
      code: `${kind}-${operation}-failed`,
      message: cause instanceof Error ? cause.message : String(cause),
      recoverable: true,
      cause,
    });
  }
  function connectionChanged(value: ConversationState['connection']) {
    dispatch({ type: 'connection', state: value });
    emit('agent-conversation-connection-change', {
      transportId: transport?.id ?? 'unconfigured',
      state: value,
    });
  }

  function rejectDeferred(reason: string) {
    const pending = deferred;
    deferred = null;
    pending?.cleanup();
    pending?.resolve({ accepted: false, rejectReason: reason });
  }

  function dispatch(action: ConversationAction) {
    const next = conversationReducer(state, action);
    if (next === state) return;
    state = next;
    composerSession.sync(state, transport, composerController, state.modelUid);
    conversationId = state.conversationId;
    emit('agent-conversation-state-change', { state, reason: action.type });
    if (action.type === 'snapshot' && state.runtimeState === 'waiting')
      pausing = false;
    if (
      state.initialized &&
      state.snapshotReceived &&
      !state.awaitingAssistantResponse &&
      state.runtimeState === 'waiting' &&
      !pausing
    ) {
      const pending = deferred;
      deferred = null;
      if (pending) {
        pending.cleanup();
        void send(pending.content, pending.options).then(pending.resolve);
      }
    }
    followReveal();
  }

  export async function connect() {
    connectAbort?.abort();
    const epoch = ++generation;
    permissionOperation = '';
    permissionError = '';
    const previous = connection;
    connection = null;
    await previous?.disconnect();
    if (epoch !== generation) return;
    rejectDeferred('Conversation connection changed.');
    if (!transport) return;
    connectAbort = new AbortController();
    pausing = false;
    dispatch({ type: 'reset', conversationId });
    connectionChanged('connecting');
    try {
      const connected = await transport.connect(
        { conversationId, locale, signal: connectAbort.signal },
        {
          event: (value) => {
            if (epoch === generation) handleEvent(value);
          },
          connection: (value) => {
            if (epoch === generation) connectionChanged(value);
          },
          error: (value) => {
            if (epoch === generation) handleError(value);
          },
        },
      );
      if (epoch !== generation) {
        await connected.disconnect();
        return;
      }
      connection = connected;
    } catch (cause) {
      if (epoch !== generation) return;
      rejectDeferred('Connection failed.');
      handleError({
        code: 'connect-failed',
        message: cause instanceof Error ? cause.message : String(cause),
        recoverable: true,
      });
      connectionChanged('disconnected');
    }
  }

  export function disconnect() {
    composerSession.dispose();
    composerPanel = null;
    ++generation;
    connectAbort?.abort();
    connectAbort = undefined;
    const previous = connection;
    connection = null;
    void previous?.disconnect();
    rejectDeferred('Conversation disconnected.');
    connectionChanged('disconnected');
  }

  function handleEvent(event: ConversationTransportEvent) {
    if (event.type === 'conversation-created') {
      dispatch({
        type: 'conversation-created',
        conversationId: event.conversationId,
        eventSeq: event.eventSeq,
      });
      emit('agent-conversation-ready', {
        conversationId: event.conversationId,
        state,
      });
    } else if (event.type === 'conversation-closed') {
      dispatch({
        type: 'conversation-closed',
        conversationId: event.conversationId,
      });
    } else if (event.type === 'state-snapshot') {
      if (
        event.conversationId &&
        state.conversationId &&
        event.conversationId !== state.conversationId
      )
        return;
      const first = !state.snapshotReceived;
      dispatch({
        type: 'snapshot',
        payload: event.payload,
        eventSeq: event.eventSeq,
      });
      if (first && event.payload.conversation_state === 'waiting')
        completedReveal = new Set([
          ...completedReveal,
          ...snapshotRecords(event.payload)
            .filter((r) => r.role === 'assistant')
            .map(recordKey),
        ]);
    } else if (event.type === 'pending-user-message') {
      if (event.conversationId !== state.conversationId) return;
      if (
        state.pendingUserMessages.some(
          (message) => message.id === event.messageId,
        )
      )
        return;
      dispatch({
        type: 'local-message-added',
        message: createPendingUserMessage(
          event.messageId,
          event.content,
          event.createdAt,
        ),
      });
      dispatch({ type: 'local-message-accepted', id: event.messageId });
    } else if (event.type === 'transport-extension') {
      // The event is public even when the host has no optional extension renderer.
      void extensions;
      if (event.extension.kind === 'tool:permission-resolved') {
        const payload = event.extension.payload as {
          tool_call_id?: unknown;
        } | null;
        if (typeof payload?.tool_call_id === 'string') {
          if (permissionErrorId === payload.tool_call_id) permissionError = '';
          dispatch({
            type: 'tool-permission-resolved',
            toolCallId: payload.tool_call_id,
          });
        }
      }
      emit('agent-conversation-extension-action', {
        extension: event.extension,
        action: 'transport-event',
      });
    }
  }

  function handleError(error: ConversationTransportError) {
    state = { ...state, lastError: error.message };
    emit('agent-conversation-error', error);
  }

  export async function send(
    content = draft,
    options: SendOptions = {},
  ): Promise<SendResult> {
    let value = content.trim();
    const images = [...pendingImages];
    if ((!value && !images.length) || !transport || options.signal?.aborted)
      return {
        accepted: false,
        rejectReason: 'Message is empty, aborted, or transport unavailable.',
      };
    if (!state.initialized || !state.snapshotReceived || pausing) {
      if (deferred)
        return {
          accepted: false,
          rejectReason: 'A message is already waiting.',
        };
      return new Promise((resolve) => {
        const abort = () => rejectDeferred('Message send aborted.');
        options.signal?.addEventListener('abort', abort, { once: true });
        deferred = {
          content: value,
          options,
          resolve,
          cleanup: () => options.signal?.removeEventListener('abort', abort),
        };
      });
    }
    if (
      !state.conversationId ||
      state.runtimeState !== 'waiting' ||
      state.awaitingAssistantResponse ||
      imageSending ||
      !composerSession.canCompose ||
      operation === 'model'
    )
      return {
        accepted: false,
        rejectReason: 'Conversation is still running.',
      };
    const targetId = state.conversationId;
    const epoch = generation;
    const sendingTransport = transport;
    imageSending = true;
    let parts: MessagePart[] | undefined;
    try {
      if (images.length) {
        if (!sendingTransport.imageInput)
          throw new Error('Image input is not supported by this host.');
        const imported = new Map<number, string>();
        for (const image of images) {
          if (options.signal?.aborted) throw new Error('Message send aborted.');
          const result = await sendingTransport.imageInput.importImage({
            conversationId: targetId,
            file: image.file,
            signal: options.signal,
          });
          if (!result.imageId)
            throw new Error(`Image import returned no ID: ${image.file.name}`);
          imported.set(image.number, result.imageId);
        }
        parts = [];
        const used = new Set<number>();
        let cursor = 0;
        let count = 0;
        for (const match of value.matchAll(/@图片(\d+)/g)) {
          const number = Number(match[1]),
            imageId = imported.get(number);
          if (!imageId) continue;
          const index = match.index ?? 0;
          if (index > cursor)
            parts.push({ type: 'text', text: value.slice(cursor, index) });
          parts.push({ type: 'image', image_id: imageId });
          used.add(number);
          count++;
          cursor = index + match[0].length;
        }
        if (cursor < value.length)
          parts.push({ type: 'text', text: value.slice(cursor) });
        for (const image of images)
          if (!used.has(image.number)) {
            parts.push({
              type: 'image',
              image_id: imported.get(image.number)!,
            });
            count++;
          }
        if (count > 8)
          throw new Error(
            'At most 8 image references can be sent in one message.',
          );
        value = parts
          .filter(
            (part): part is Extract<MessagePart, { type: 'text' }> =>
              part.type === 'text',
          )
          .map((p) => p.text)
          .join('\n');
        if (!value.trim()) value = '[Image]';
      }
      if (
        epoch !== generation ||
        options.signal?.aborted ||
        !state.initialized ||
        !state.snapshotReceived ||
        state.runtimeState !== 'waiting' ||
        state.awaitingAssistantResponse
      )
        throw new Error(
          'Conversation changed or send aborted while importing images.',
        );
    } catch (cause) {
      imageSending = false;
      const message = cause instanceof Error ? cause.message : String(cause);
      emit('agent-conversation-error', {
        code: 'image-import-failed',
        message,
        recoverable: true,
      });
      return { accepted: false, rejectReason: message };
    }
    const id = `svelte-${Date.now()}-${++localSequence}`;
    dispatch({
      type: 'local-message-added',
      message: createPendingUserMessage(id, value, undefined, parts),
    });
    stickToBottom = true;
    try {
      const result = await sendingTransport.send({
        conversationId: targetId,
        content: value,
        parts,
        clientMessageId: id,
        signal: options.signal,
        metadata: { ...options.metadata, source: options.source },
      });
      if (epoch === generation)
        dispatch(
          result.accepted
            ? { type: 'local-message-accepted', id }
            : {
                type: 'local-message-failed',
                id,
                error: result.rejectReason ?? 'Message rejected.',
              },
        );
      if (result.accepted && epoch === generation) {
        composerSession.sent();
        draft = '';
        clearPendingImages();
        if (composerElement) composerElement.style.height = '';
      }
      emit('agent-conversation-send', {
        conversationId: targetId,
        clientMessageId: id,
        content: value,
        source: options.source,
        result,
      });
      return result;
    } catch (cause) {
      const error = cause instanceof Error ? cause.message : String(cause);
      if (epoch === generation)
        dispatch({ type: 'local-message-failed', id, error });
      const result = { accepted: false, rejectReason: error };
      emit('agent-conversation-send', {
        conversationId: targetId,
        clientMessageId: id,
        content: value,
        source: options.source,
        result,
      });
      return result;
    } finally {
      imageSending = false;
    }
  }

  export async function pause(): Promise<CommandResult> {
    if (
      !state.conversationId ||
      !transport?.pause ||
      permissionOperation ||
      imageSending
    )
      return { accepted: false, rejectReason: 'Pause is not supported.' };
    const targetId = state.conversationId;
    const epoch = generation;
    pausing = true;
    try {
      const result = await composerSession.command('pause');
      if (!result.accepted && epoch === generation) {
        pausing = false;
        rejectDeferred(result.rejectReason ?? 'Pause rejected.');
      }
      emit('agent-conversation-pause', { conversationId: targetId, result });
      return result;
    } catch (cause) {
      if (epoch === generation) {
        pausing = false;
        rejectDeferred('Pause failed.');
      }
      throw cause;
    }
  }

  export async function resume(): Promise<CommandResult> {
    const targetId = state.conversationId;
    if (!targetId)
      return { accepted: false, rejectReason: 'Conversation is not ready.' };
    const result = await composerSession.command('resume');
    emit('agent-conversation-resume', { conversationId: targetId, result });
    return result;
  }

  export async function openConversation(id: string) {
    clearPendingImages();
    completedReveal = new Set();
    revealLengths.clear();
    conversationId = id;
    await connect();
  }

  export async function createConversation() {
    if (persistenceController?.create) {
      const controller = persistenceController;
      const epoch = generation;
      operation = 'create';
      try {
        const binding = await controller.create!();
        if (controller !== persistenceController || epoch !== generation)
          return;
        if (persistence.enabled) persistence = { ...persistence, binding };
        historyOpen = false;
        await openConversation(binding.runtimeConversationId);
      } catch (cause) {
        report('create', cause);
      } finally {
        operation = '';
      }
      return;
    }
    if (persistenceController)
      emit('agent-conversation-diagnostic', {
        code: 'conversation-create-unsupported',
        message:
          'The host persistence controller does not support creating conversations.',
      });
  }

  export async function saveConversation() {
    if (
      !persistenceController ||
      !state.conversationId ||
      state.runtimeState !== 'waiting'
    )
      return null;
    const controller = persistenceController;
    const targetId = state.conversationId;
    const epoch = generation;
    operation = 'save';
    try {
      const saved = await controller.save({
        archiveId: persistence.enabled
          ? persistence.binding?.archiveId
          : undefined,
        runtimeConversationId: targetId,
      });
      if (controller !== persistenceController || epoch !== generation)
        return saved;
      if (persistence.enabled)
        persistence = {
          ...persistence,
          binding: {
            archiveId: saved.archiveId,
            runtimeConversationId:
              saved.runtimeConversationId ?? state.conversationId,
          },
        };
      archives = [
        saved,
        ...archives.filter((a) => a.archiveId !== saved.archiveId),
      ];
      return saved;
    } catch (cause) {
      report('save', cause);
      return null;
    } finally {
      operation = '';
    }
  }

  export async function restoreConversation(archiveId: string) {
    if (!persistenceController) return;
    const controller = persistenceController;
    const epoch = generation;
    operation = `restore:${archiveId}`;
    try {
      const binding = await controller.restore({ archiveId });
      if (controller !== persistenceController || epoch !== generation) return;
      if (persistence.enabled) persistence = { ...persistence, binding };
      await openConversation(binding.runtimeConversationId);
      historyOpen = false;
    } catch (cause) {
      report('restore', cause);
    } finally {
      operation = '';
    }
  }

  export async function refreshHistory() {
    if (!persistenceController) return;
    const controller = persistenceController;
    try {
      const result = await controller.list();
      if (controller === persistenceController) archives = result.items;
    } catch (cause) {
      report('list', cause);
    }
  }

  export async function refreshConversationHistory() {
    await refreshHistory();
  }
  export async function closeConversation() {
    const id = state.conversationId;
    const epoch = generation;
    if (id && transport?.close) {
      const result = await transport.close({ conversationId: id });
      if (!result.accepted)
        throw new Error(result.rejectReason ?? 'Close rejected.');
    }
    if (epoch === generation) {
      disconnect();
      dispatch({
        type: 'conversation-closed',
        conversationId: id ?? undefined,
      });
    }
  }
  export function focusComposer() {
    composerElement?.focus();
  }
  export function insertPresetMarkdown(preset: PresetMarkdown) {
    const id = preset.id ?? `preset-${Date.now()}-${++localSequence}`;
    const last = [...state.records]
      .reverse()
      .find((r) => r.role === 'assistant' || r.role === 'user');
    insertPresentationItem({
      contract: 'agent-conversation-presentation/v1',
      id,
      kind: 'assistant-markdown',
      scope: preset.scope ?? 'preset-markdown',
      content: preset.markdown,
      reveal: 'progressive',
      anchor: last
        ? { type: 'after-record', recordId: recordKey(last) }
        : { type: 'tail' },
      metadata: { baseDir: preset.baseDir, presetName: preset.name },
      createdAt: preset.createdAt,
    });
    return id;
  }
  export function insertPresentationItem(item: ConversationPresentationItem) {
    presentationItems = [
      ...presentationItems.filter((p) => p.id !== item.id),
      item,
    ];
  }
  export function updatePresentationItem(
    id: string,
    patch: PresentationItemPatch,
  ) {
    completedReveal = new Set(
      [...completedReveal].filter((key) => key !== `presentation:${id}`),
    );
    presentationItems = presentationItems.map((item) =>
      item.id === id ? { ...item, ...patch } : item,
    );
  }
  export function removePresentationItem(id: string) {
    presentationItems = presentationItems.filter((item) => item.id !== id);
  }
  export function clearPresentationItems(scope?: string) {
    presentationItems = scope
      ? presentationItems.filter((item) => item.scope !== scope)
      : [];
  }

  export async function refreshProviders() {
    if (!providerController) return;
    const controller = providerController;
    try {
      const result = await controller.getProviderDefinitions();
      if (controller !== providerController) return;
      providers = result;
      emit('agent-conversation-provider-action', {
        action: 'definitions-loaded',
        definitions: providers,
      });
    } catch (cause) {
      report('load', cause, 'provider');
    }
  }

  export async function setConversationModel(modelUid: number) {
    if (
      !providerController ||
      !state.conversationId ||
      !composerSession.canConfigure ||
      imageSending ||
      operation
    )
      return;
    const target = state.conversationId;
    const epoch = generation;
    operation = 'model';
    try {
      const result = await providerController.setConversationModel({
        conversationId: state.conversationId,
        modelUid,
      });
      if (result.accepted === false)
        throw new Error(result.rejectReason ?? 'Model switch was rejected');
      if (epoch === generation && target === state.conversationId) {
        const model = providers?.models?.find((item) => item.uid === modelUid);
        dispatch({
          type: 'conversation-model-selected',
          model: modelName(model),
          modelUid,
        });
        emit('agent-conversation-provider-action', {
          action: 'model-selected',
          modelUid,
          result,
        });
      }
    } catch (cause) {
      report('model', cause, 'provider');
    } finally {
      operation = '';
    }
  }

  export async function resolveToolPermission(
    toolCallId: string,
    decision: 'allow' | 'deny',
  ): Promise<CommandResult> {
    if (
      permissionOperation ||
      state.connection !== 'connected' ||
      composerSession.awaiting ||
      !state.pendingPermissions.some((p) => p.tool_call_id === toolCallId)
    )
      return {
        accepted: false,
        rejectReason:
          'Permission response is unavailable in the current state.',
      };
    if (!transport?.resolveToolPermission || !state.conversationId)
      return {
        accepted: false,
        rejectReason: 'Permission response unsupported.',
      };
    const targetId = state.conversationId;
    const epoch = generation;
    permissionOperation = toolCallId;
    permissionErrorId = toolCallId;
    permissionError = '';
    try {
      const result = await transport.resolveToolPermission({
        conversationId: state.conversationId,
        toolCallId,
        decision,
      });
      if (epoch !== generation || state.conversationId !== targetId)
        return result;
      if (
        !result.accepted &&
        (result.metadata?.resolved === false ||
          result.rejectReason === 'Permission request is no longer pending.')
      ) {
        dispatch({ type: 'tool-permission-resolved', toolCallId });
        await transport.requestSnapshot?.({ conversationId: targetId });
      } else if (!result.accepted)
        permissionError =
          result.rejectReason ?? 'Request is no longer pending.';
      else dispatch({ type: 'tool-permission-resolved', toolCallId });
      emit('agent-conversation-tool-permission', {
        conversationId: state.conversationId!,
        toolCallId,
        decision,
        result,
      });
      return result;
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      if (epoch === generation) {
        permissionError = message;
        report('response', cause, 'tool-permission');
      }
      return { accepted: false, rejectReason: message };
    } finally {
      if (epoch === generation) permissionOperation = '';
    }
  }

  function modelName(model: ProviderModelDefinition | undefined): string {
    return (
      model?.model_name ??
      model?.modelName ??
      model?.model_id ??
      model?.modelId ??
      model?.name ??
      ''
    );
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void send();
    }
  }

  onMount(() => {
    const media = globalThis.matchMedia?.('(prefers-color-scheme: dark)');
    mediaDark = media?.matches ?? false;
    const changed = (event: MediaQueryListEvent) => (mediaDark = event.matches);
    media?.addEventListener?.('change', changed);
    const dismiss = (event: Event) => {
      const path = event.composedPath();
      if (
        !path.some(
          (node) =>
            node instanceof Element &&
            (node.classList.contains('composer-menu') ||
              node.matches(
                '[aria-label="Permissions"],[aria-label="Model and reasoning"]',
              )),
        )
      )
        composerPanel = null;
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') composerPanel = null;
    };
    const closePanel = () => (composerPanel = null);
    window.addEventListener('pointerdown', dismiss);
    window.addEventListener('keydown', escape);
    window.addEventListener('resize', closePanel);
    void connect();
    return () => {
      composerSession.dispose();
      window.removeEventListener('pointerdown', dismiss);
      window.removeEventListener('keydown', escape);
      window.removeEventListener('resize', closePanel);
      disconnect();
      unsubscribePersistence?.();
      clearPendingImages();
      media?.removeEventListener?.('change', changed);
    };
  });

  function onDraftInput(event: Event): void {
    const textarea = event.target as HTMLTextAreaElement;
    draft = textarea.value;
    const retained = pendingImages.filter((image) =>
      draft.includes(`@图片${image.number}`),
    );
    for (const image of pendingImages) {
      if (!retained.includes(image) && image.previewUrl)
        URL.revokeObjectURL(image.previewUrl);
    }
    if (retained.length !== pendingImages.length) {
      pendingImages = retained;
      renumberPendingImages();
    }
    textarea.style.height = '0px';
    textarea.style.height = `${Math.min(textarea.scrollHeight, 180)}px`;
  }

  function onImageInput(event: Event): void {
    const input = event.target as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    addPendingImages(files);
  }

  function onComposerPaste(event: ClipboardEvent): void {
    if (!transport?.imageInput || !canCompose || imageSending) return;
    const items = Array.from(event.clipboardData?.items ?? []);
    const itemFiles = items
      .filter((item) => item.kind === 'file')
      .map((item) => item.getAsFile())
      .filter(
        (file): file is File => file !== null && file.type.startsWith('image/'),
      );
    const files = itemFiles.length
      ? itemFiles
      : Array.from(event.clipboardData?.files ?? []).filter((file) =>
          file.type.startsWith('image/'),
        );
    if (!files.length) return;
    event.preventDefault();
    const text = event.clipboardData?.getData?.('text/plain') ?? '';
    addPendingImages(files, text);
  }

  function addPendingImages(files: File[], pastedText = ''): void {
    const markers: string[] = [];
    for (const file of files) {
      if (
        !['image/png', 'image/jpeg', 'image/webp', 'image/gif'].includes(
          file.type,
        ) ||
        file.size > 20 * 1024 * 1024 ||
        pendingImages.length >= 8
      ) {
        emit('agent-conversation-error', {
          code: 'invalid-image',
          message: `Unsupported image, image larger than 20 MiB, or more than 8 images: ${file.name}`,
          recoverable: true,
        });
        continue;
      }
      const previewUrl =
        typeof URL.createObjectURL === 'function'
          ? URL.createObjectURL(file)
          : '';
      const number =
        pendingImages.reduce((max, image) => Math.max(max, image.number), 0) +
        1;
      pendingImages = [...pendingImages, { file, previewUrl, number }];
      markers.push(`@图片${number}`);
    }
    if (pastedText || markers.length) {
      insertIntoDraft([pastedText, ...markers].filter(Boolean).join(' '));
    }
  }

  function insertIntoDraft(value: string): void {
    const textarea = composerElement;
    const start = textarea?.selectionStart ?? draft.length;
    const end = textarea?.selectionEnd ?? draft.length;
    const before = draft.slice(0, start);
    const after = draft.slice(end);
    const prefix = /@图片\d+$/.test(before) && /^\S/.test(value) ? ' ' : '';
    const suffix =
      /@图片\d+$/.test(value) && /^[A-Za-z0-9]/.test(after) ? ' ' : '';
    draft = `${before}${prefix}${value}${suffix}${after}`;
    void tick().then(() => {
      const current = composerElement;
      const cursor = start + prefix.length + value.length + suffix.length;
      current?.setSelectionRange(cursor, cursor);
    });
  }

  function removePendingImage(image: PendingImage, removeMarker = true): void {
    pendingImages = pendingImages.filter((item) => item !== image);
    if (image.previewUrl) URL.revokeObjectURL(image.previewUrl);
    if (removeMarker) {
      const label = `@图片${image.number}`;
      const index = draft.indexOf(label);
      if (index >= 0) {
        let before = draft.slice(0, index);
        let after = draft.slice(index + label.length);
        if (before.endsWith(' ') && after.startsWith(' '))
          before = before.slice(0, -1);
        else if (!before && after.startsWith(' ')) after = after.slice(1);
        draft = before + after;
      }
    }
    renumberPendingImages();
  }

  function renumberPendingImages(): void {
    const numbers = new Map(
      pendingImages.map((image, index) => [image.number, index + 1]),
    );
    draft = draft.replace(/@图片(\d+)/g, (marker, number: string) => {
      const next = numbers.get(Number(number));
      return next ? `@图片${next}` : marker;
    });
    pendingImages = pendingImages.map((image, index) => ({
      ...image,
      number: index + 1,
    }));
  }

  function clearPendingImages(): void {
    for (const image of pendingImages) {
      if (image.previewUrl) URL.revokeObjectURL(image.previewUrl);
    }
    pendingImages = [];
  }

  async function openEditor(uid: number | null) {
    if (operation) return;
    composerPanel = null;
    editorProviderUid = uid;
    providerError = '';
    if (!providers) await refreshProviders();
    editorOpen = true;
    providerOpen = false;
    historyOpen = false;
  }

  function modelLabel(model: ProviderModelDefinition) {
    const uid = model.provider_uid ?? model.providerUid;
    const provider = providers?.providers?.find((p) => p.uid === uid);
    return provider?.name
      ? `${modelName(model)} / ${provider.name}`
      : modelName(model);
  }
  function currentModelLabel(
    state: ConversationState,
    providers: ProviderDefinitionsResult | null,
    zh: boolean,
  ) {
    const current = providers?.models?.find((m) =>
      state.modelUid !== undefined
        ? m.uid === state.modelUid
        : modelName(m) === state.model,
    );
    return current
      ? modelLabel(current)
      : state.model || (zh ? '未配置模型' : 'No model');
  }
  function formatArchiveTime(value: string | undefined, locale: string) {
    if (!value) return '';
    const date = /^\d+$/.test(value)
      ? new Date(Number(value))
      : new Date(value);
    return Number.isNaN(date.getTime())
      ? value
      : new Intl.DateTimeFormat(locale, {
          month: '2-digit',
          day: '2-digit',
          hour: '2-digit',
          minute: '2-digit',
        }).format(date);
  }
  function isCurrentArchive(
    archive: PersistedConversation,
    persistence: ConversationPersistencePolicy,
    conversationId: string | null,
  ) {
    const binding = persistence.enabled ? persistence.binding : null;
    return Boolean(
      binding?.archiveId
        ? binding.archiveId === archive.archiveId
        : archive.runtimeConversationId &&
            archive.runtimeConversationId === conversationId,
    );
  }
  function activityLabel(state: ConversationState, zh: boolean) {
    if (state.runtimeState === 'compacting') return '总结中';
    if (state.runtimeState === 'waiting') return null;
    if (
      state.runtimeState === 'executing' ||
      state.toolCalls.some((c) =>
        ['placeholder', 'running', 'waiting_permission'].includes(c.status),
      )
    )
      return zh ? '工具执行中' : 'Running tools';
    if (state.turnObservedAssistant) return null;
    return 'Loading';
  }
</script>

<div
  class="orbit-svelte-conversation"
  data-theme={theme}
  data-color-scheme={colorScheme}
  data-density={density}
  lang={locale}
>
  <section class="shell" part="shell">
    <header class="header" part="header">
      <slot name="header"
        ><div class="status">
          <span class="header-title">{zh ? '对话' : 'Chat'}</span><span
            class="connection-label"
            role="status"
            >{state.connection === 'reconnecting'
              ? zh
                ? '重新连接中'
                : 'Reconnecting'
              : state.connection === 'connecting'
                ? zh
                  ? '连接中'
                  : 'Connecting'
                : state.connection !== 'connected'
                  ? zh
                    ? '未连接'
                    : 'Offline'
                  : !state.initialized
                    ? zh
                      ? '初始化中'
                      : 'Initializing'
                    : zh
                      ? '已连接'
                      : 'Connected'}</span
          >
        </div></slot
      >
      <div class="header-control-strip">
        {#if providerController}
          <div class="header-actions" part="provider-actions">
            {#if providerControls.enabled && providerControls.showImport !== false}<button
                class="header-action"
                data-tone="add"
                on:click={() => void openEditor(null)}
                ><svg viewBox="0 0 24 24" aria-hidden="true"
                  ><path
                    d="m12 3 8 4-8 4-8-4zM4 12l8 4 8-4M4 17l8 4 8-4"
                  /></svg
                ><span>{zh ? '添加厂商' : 'Add provider'}</span></button
              >{/if}
          </div>
        {/if}
        {#if persistenceController}
          <div class="header-actions" part="persistence-actions">
            <button
              class="header-action"
              data-tone="new"
              disabled={Boolean(operation)}
              on:click={() => void createConversation()}
              ><svg viewBox="0 0 24 24" aria-hidden="true"
                ><path d="M12 5v14M5 12h14" /></svg
              ><span>{zh ? '新会话' : 'New chat'}</span></button
            >
            {#if persistence.enabled && persistence.showHistory !== false}<button
                class="header-action"
                data-tone="history"
                data-active={String(historyOpen)}
                aria-expanded={historyOpen}
                on:click={() => {
                  historyOpen = !historyOpen;
                  if (historyOpen) {
                    providerOpen = false;
                    void refreshHistory();
                  }
                }}
                ><svg viewBox="0 0 24 24" aria-hidden="true"
                  ><path d="M3 11a9 9 0 1 1 2.6 7M3 5v6h6M12 7v5l3 2" /></svg
                ><span>{zh ? '历史会话' : 'History'}</span></button
              >{/if}
          </div>
        {/if}
      </div>
    </header>
    {#if providerOpen && providerController}
      <aside class="provider-panel" part="provider-panel">
        {#if providers?.models?.length}
          <label class="provider-field"
            ><span class="provider-label"
              >{zh ? '当前模型' : 'Current model'}</span
            ><select
              class="provider-select"
              value={state.modelUid ?? ''}
              disabled={Boolean(operation) || !composer.canConfigure}
              on:change={(event) =>
                void setConversationModel(Number(event.currentTarget.value))}
            >
              <option value="" disabled
                >{zh ? '选择模型' : 'Choose model'}</option
              >{#each providers.models as model}<option value={model.uid}
                  >{modelLabel(model)}</option
                >{/each}
            </select></label
          >
          <div class="provider-list">
            {#each providers.providers ?? [] as provider}<div
                class="provider-list-item"
              >
                <div class="provider-list-main">
                  <strong>{provider.name ?? `Provider ${provider.uid}`}</strong
                  ><span
                    >{providers.models.filter(
                      (m) => (m.provider_uid ?? m.providerUid) === provider.uid,
                    ).length}
                    {zh ? '个模型' : 'models'}</span
                  >
                </div>
                <button
                  class="secondary"
                  disabled={Boolean(operation)}
                  on:click={() => void openEditor(provider.uid)}
                  >{zh ? '编辑' : 'Edit'}</button
                >
              </div>{/each}
          </div>
          <div class="provider-panel-actions">
            <button class="secondary" on:click={() => void openEditor(null)}
              >{zh ? '添加厂商' : 'Add provider'}</button
            ><button class="secondary" on:click={() => void refreshProviders()}
              >{zh ? '刷新' : 'Refresh'}</button
            >
          </div>
        {:else}<div class="provider-empty">
            {zh ? '未配置厂商模型' : 'No provider models configured'}
            <div class="provider-panel-actions">
              <button class="secondary" on:click={() => void openEditor(null)}
                >{zh ? '添加厂商' : 'Add provider'}</button
              >
            </div>
          </div>{/if}
      </aside>
    {/if}
    {#if editorOpen && providerController && providers}
      <div class="provider-editor-backdrop" role="presentation">
        {#key editorProviderUid}<ProviderEditor
            controller={providerController}
            definitions={providers}
            providerUid={editorProviderUid}
            {locale}
            on:cancel={() => (editorOpen = false)}
            on:saved={() => {
              editorOpen = false;
              void refreshProviders();
            }}
          />{/key}
      </div>
    {/if}
    {#if historyOpen && persistenceController}
      <aside class="history-panel" part="history-panel">
        {#if !archives.length}<div class="history-empty">
            {zh ? '还没有会话' : 'No conversations'}
          </div>{/if}
        {#each archives as archive}
          {@const current = isCurrentArchive(
            archive,
            persistence,
            state.conversationId,
          )}
          <div class="history-item" data-current={String(current)}>
            <div class="history-copy">
              <div class="history-title">
                {archive.title ?? (zh ? '未命名会话' : 'Untitled conversation')}
              </div>
              <div class="history-preview">
                {archive.preview ?? archive.updatedAt ?? archive.archiveId}
              </div>
              <div class="history-meta">
                {current || archive.instanceState === 'current'
                  ? zh
                    ? '当前会话'
                    : 'Current conversation'
                  : archive.instanceState === 'background'
                    ? zh
                      ? '后台运行'
                      : 'Running in background'
                    : zh
                      ? '仅数据'
                      : 'Saved data'} · {formatArchiveTime(
                  archive.updatedAt ?? archive.createdAt,
                  locale,
                )}
              </div>
            </div>
            <button
              disabled={current || Boolean(operation)}
              on:click={() => void restoreConversation(archive.archiveId)}
              >{current
                ? zh
                  ? '当前'
                  : 'Current'
                : operation === `restore:${archive.archiveId}`
                  ? zh
                    ? '打开中'
                    : 'Opening'
                  : zh
                    ? '打开'
                    : 'Open'}</button
            >
          </div>
        {/each}
      </aside>
    {/if}
    <main
      class="messages"
      part="message-list"
      bind:this={messagesElement}
      aria-live="polite"
      on:scroll={() =>
        (stickToBottom =
          messagesElement.scrollHeight -
            messagesElement.scrollTop -
            messagesElement.clientHeight <=
          40)}
      on:wheel={() => (stickToBottom = false)}
    >
      {#if !displayItems.length}<div class="empty">
          <slot name="empty-state">Start a conversation</slot>
        </div>{/if}
      {#each displayItems as item (item.key)}
        {#if item.kind === 'presentation'}
          {#if item.item.kind === 'assistant-markdown' || item.item.kind === 'notice'}<article
              class="message assistant"
              part="assistant-message"
            >
              <RichContent
                content={item.item.content ?? ''}
                toolCalls={state.toolCalls}
                {dark}
                {hideToolCalls}
                {capabilities}
                {locale}
                widgetsExpired
                baseDir={typeof item.item.metadata?.baseDir === 'string'
                  ? item.item.metadata.baseDir
                  : undefined}
                contentMetadata={item.item.metadata}
                reveal={item.item.reveal === 'progressive' &&
                  !completedReveal.has(item.key)}
                on:complete={() => completeReveal(item.key)}
                on:reveal={followReveal}
              />
            </article>{/if}
        {:else if item.record.role === 'user'}<article
            class="message user"
            part="user-message"
          >
            <div class="user-bubble">
              <UserRecord
                record={item.record}
                {capabilities}
                {locale}
                conversationId={state.conversationId ?? undefined}
                on:error={(event) =>
                  emit('agent-conversation-error', event.detail)}
              />
            </div>
          </article>
        {:else}
          {@const reveal =
            item.record.role === 'assistant' &&
            progressiveReveal &&
            item.key === latestAssistant &&
            !completedReveal.has(item.key)}
          <article class="message assistant" part="assistant-message">
            <RichContent
              content={item.record.role === 'gateway_message'
                ? displayText(item.record)
                : displayTextWithToolAnchors(item.record)}
              toolCalls={state.toolCalls}
              {dark}
              {hideToolCalls}
              {capabilities}
              {locale}
              {reveal}
              widgetsExpired={item.key !== latestWidget}
              on:complete={() => completeReveal(item.key)}
              on:reveal={followReveal}
              on:submit={(event) =>
                void send(event.detail.content, { source: 'widget' })}
            />
            {#if disclaimer && item.key === latestAssistant && state.runtimeState === 'waiting' && !reveal}<div
                class="disclaimer"
                part="disclaimer"
              >
                {disclaimer}
              </div>{/if}
          </article>
        {/if}
      {/each}
      {#if state.plan}<PlanCard plan={state.plan} {locale} />{/if}
      {#if !state.assistantStream?.content && activityLabel(state, zh)}<div
          class="message assistant waiting"
        >
          <span class="pulse"></span><span>{activityLabel(state, zh)}</span>
        </div>{/if}
      {#if state.lastError}<div class="error" role="alert">
          {state.lastError}
        </div>{/if}
    </main>
    {#if state.pendingPermissions.length}
      <div class="permission-shelf-wrap">
        <section
          class="permission-shelf approval"
          part="permission-shelf"
          aria-live="polite"
          aria-label={zh ? '工具批准请求' : 'Tool approval requests'}
        >
          <div class="permission-shelf-header">
            {state.pendingPermissions.length === 1
              ? zh
                ? '等待批准'
                : 'Waiting for approval'
              : zh
                ? `${state.pendingPermissions.length} 个请求等待批准`
                : `${state.pendingPermissions.length} requests waiting for approval`}
          </div>
          <div class="permission-tools">
            {#each state.pendingPermissions as permission}
              {@const busy = permissionOperation === permission.tool_call_id}
              <div
                class="permission-tool permission-item"
                data-call-id={permission.tool_call_id}
              >
                <div class="permission-copy">
                  <div class="permission-tool-name">
                    {permission.display_name || permission.tool_name}
                  </div>
                  <div class="permission-effect">
                    {permission.effect === 'destructive'
                      ? zh
                        ? '破坏性操作'
                        : 'Destructive operation'
                      : permission.effect === 'controlled_change'
                        ? zh
                          ? '可控变更'
                          : 'Controlled change'
                        : zh
                          ? '只读操作'
                          : 'Read-only operation'} · {permission.tool_name}
                  </div>
                  {#if summarizePermissionTargets(permission).length}
                    <div class="permission-arguments">
                      {#each summarizePermissionTargets(permission) as target}<div
                          class="permission-argument"
                        >
                          <span class="permission-argument-label"
                            >{target.label}</span
                          ><span>{target.value}</span>
                        </div>{/each}
                    </div>
                  {/if}
                  {#if permissionError && permissionErrorId === permission.tool_call_id}<div
                      class="permission-error"
                      role="alert"
                    >
                      {permissionError}
                    </div>{/if}
                </div>
                <div class="permission-actions">
                  <button
                    class="deny"
                    type="button"
                    disabled={Boolean(permissionOperation) ||
                      state.connection !== 'connected' ||
                      Boolean(composer.awaiting)}
                    on:click={() =>
                      void resolveToolPermission(
                        permission.tool_call_id,
                        'deny',
                      )}>{zh ? '拒绝' : 'Deny'}</button
                  >
                  <button
                    type="button"
                    disabled={Boolean(permissionOperation) ||
                      state.connection !== 'connected' ||
                      Boolean(composer.awaiting)}
                    on:click={() =>
                      void resolveToolPermission(
                        permission.tool_call_id,
                        'allow',
                      )}
                    >{busy
                      ? zh
                        ? '处理中'
                        : 'Working'
                      : zh
                        ? '允许'
                        : 'Allow'}</button
                  >
                </div>
              </div>
            {/each}
          </div>
        </section>
      </div>
    {/if}
    <footer class="composer-wrap" part="composer">
      <div class="composer">
        <slot name="composer-prefix" />
        {#if pendingImages.length}<div
            class="image-attachments"
            aria-label="Selected images"
          >
            {#each pendingImages as image}<span class="image-attachment"
                >{#if image.previewUrl}<img
                    src={image.previewUrl}
                    alt=""
                  />{/if}<span title={image.file.name}>{image.file.name}</span
                ><button
                  class="secondary remove-image"
                  type="button"
                  aria-label={`Remove ${image.file.name}`}
                  disabled={imageSending}
                  on:click={() => removePendingImage(image)}>×</button
                ></span
              >{/each}
          </div>{/if}
        <textarea
          rows="1"
          aria-label="Message"
          bind:this={composerElement}
          value={draft}
          on:input={onDraftInput}
          on:keydown={onKeydown}
          on:paste={onComposerPaste}
          disabled={!canCompose || imageSending}
          placeholder={state.initialized
            ? 'Message the agent'
            : 'Waiting for conversation'}
        ></textarea>
        {#if composerStateLabels[composerPhase]}<div
            class="composer-state"
            role="status"
          >
            {composerStateLabels[composerPhase][zh ? 0 : 1]}
          </div>{/if}
        <div class="composer-toolbar" data-state={composerPhase}>
          <div class="composer-left">
            {#if transport?.imageInput}<input
                class="image-input"
                bind:this={imageInput}
                type="file"
                accept="image/png,image/jpeg,image/webp,image/gif"
                multiple
                on:change={onImageInput}
              /><button
                class="icon-button"
                type="button"
                aria-label="Attach images"
                title={zh ? '添加图片' : 'Attach images'}
                disabled={!canCompose ||
                  imageSending ||
                  pendingImages.length >= 8}
                on:click={() => imageInput?.click()}
                ><svg viewBox="0 0 24 24" aria-hidden="true"
                  ><path d="M12 5v14M5 12h14" /></svg
                ></button
              >{/if}
            <button
              class="text-button"
              type="button"
              aria-label="Permissions"
              aria-expanded={composerPanel === 'permissions'}
              title={zh ? '工具权限' : 'Tool permissions'}
              on:click={(event) => toggleComposerPanel('permissions', event)}
              ><svg viewBox="0 0 24 24" aria-hidden="true"
                ><path d="m12 3 8 4v5c0 4-3 7-8 9-5-2-8-5-8-9V7z" /><path
                  d="m8 12 3 3 5-6"
                /></svg
              ><span
                >{composerPermission?.label ??
                  (zh ? '权限' : 'Permissions')}{state.pendingPermissions.length
                  ? ` · ${state.pendingPermissions.length}`
                  : ''}</span
              ></button
            >
          </div>
          <div class="composer-right">
            <slot name="composer-suffix" />
            {#if providerControls.enabled && providerControls.showModelSwitcher !== false}<button
                class="text-button"
                data-tone="model"
                type="button"
                aria-label="Model and reasoning"
                aria-expanded={composerPanel === 'model'}
                disabled={!composer.canConfigure ||
                  Boolean(operation) ||
                  imageSending}
                title={currentModelLabel(state, providers, zh)}
                on:click={(event) => toggleComposerPanel('model', event)}
                ><span class="model-name"
                  >{composerModel
                    ? modelName(composerModel)
                    : zh
                      ? '选择模型'
                      : 'Select model'}</span
                ><span class="effort-value"
                  >{effortLabel(
                    composer.settings.effort ??
                      composerModel?.reasoning?.effort,
                    zh,
                  )}</span
                ><svg viewBox="0 0 24 24" aria-hidden="true"
                  ><path d="m8 10 4 4 4-4" /></svg
                ></button
              >{/if}
            <button
              class="primary-action"
              type="button"
              data-state={composerPhase}
              aria-label={composerActionLabel}
              title={composerActionLabel}
              disabled={composerActionDisabled}
              on:click={() =>
                void (composerBusy
                  ? pause()
                  : composerResume
                    ? resume()
                    : send())}
              ><svg
                class:action-spinner={[
                  'pausing',
                  'resuming',
                  'sending',
                  'compacting',
                ].includes(composerPhase)}
                viewBox="0 0 24 24"
                aria-hidden="true"
                >{#if ['pausing', 'resuming', 'sending', 'compacting'].includes(composerPhase)}<path
                    d="M20 12a8 8 0 1 1-8-8"
                  />{:else if composerBusy}<rect
                    x="7"
                    y="7"
                    width="10"
                    height="10"
                    rx="1"
                    fill="currentColor"
                    stroke="none"
                  />{:else if composerResume}<path
                    d="m9 5 11 7-11 7z"
                    fill="currentColor"
                    stroke="none"
                  />{:else}<path d="M12 19V5m-6 6 6-6 6 6" />{/if}</svg
              ></button
            >
          </div>
        </div>
        {#if composerPanel}<aside
            class="composer-menu"
            style={composerPanelPosition}
            aria-label={composerPanel === 'model'
              ? 'Model settings'
              : 'Permission settings'}
          >
            {#if composerPanel === 'model'}
              <div class="reasoning-control">
                <div class="reasoning-label">
                  <span
                    >{zh ? '推理强度' : 'Reasoning effort'} · {effortLabel(
                      composer.displayedEffort,
                      zh,
                    )}</span
                  ><button
                    type="button"
                    aria-label="Reset reasoning"
                    disabled={!composer.canConfigure ||
                      !composerController?.setEffort}
                    on:click={() => {
                      composerSession.clearEffortPreview();
                      void composerSession.configure('effort', null);
                    }}>{zh ? '重置' : 'Reset'}</button
                  >
                </div>
                <input
                  type="range"
                  aria-label="Reasoning effort"
                  aria-valuetext={effortLabel(composer.displayedEffort, zh)}
                  min="-1"
                  max={Math.max(-1, composerEfforts.length - 1)}
                  step="1"
                  value={composer.displayedEffort
                    ? composerEfforts.indexOf(composer.displayedEffort)
                    : -1}
                  disabled={!composer.canConfigure ||
                    !composerEfforts.length ||
                    !composerController?.setEffort}
                  on:input={(event) =>
                    composerSession.previewEffort(
                      composerEfforts[Number(event.currentTarget.value)] ??
                        null,
                    )}
                  on:change={(event) =>
                    void composerSession.configure(
                      'effort',
                      composerEfforts[Number(event.currentTarget.value)] ??
                        null,
                    )}
                />
              </div>
              <div class="composer-menu-title">
                {zh ? '选择模型' : 'Select model'}
              </div>
              <div role="menu" aria-label="Models">
                {#each providers?.models ?? [] as model}<button
                    class="menu-item"
                    type="button"
                    role="menuitemradio"
                    aria-checked={model.uid === state.modelUid}
                    disabled={!composer.canConfigure || Boolean(operation)}
                    on:click={() => void setConversationModel(model.uid)}
                    ><span>{modelName(model)}</span
                    >{#if model.uid === state.modelUid}<span aria-hidden="true"
                        >✓</span
                      >{/if}</button
                  >{/each}
              </div>
              {#if providerControls.enabled && providerControls.showImport !== false}<button
                  class="menu-item"
                  type="button"
                  on:click={() => {
                    composerPanel = null;
                    providerOpen = true;
                    historyOpen = false;
                  }}>{zh ? '管理厂商' : 'Manage providers'}</button
                >{/if}
            {:else}
              <div class="composer-menu-title">
                {zh ? '工具权限' : 'Tool permissions'}
              </div>
              <div role="menu">
                {#if composer.settings.permissionModes?.length}{#each composer.settings.permissionModes as mode}<button
                      class="menu-item"
                      type="button"
                      role="menuitemradio"
                      aria-checked={mode.id ===
                        composer.settings.permissionMode}
                      disabled={!composer.canConfigure ||
                        !composerController?.setPermissionMode}
                      on:click={() =>
                        void composerSession.configure('permission', mode.id)}
                      ><span
                        >{mode.label}{#if mode.description}<small
                            >{mode.description}</small
                          >{/if}</span
                      >{#if mode.id === composer.settings.permissionMode}<span
                          aria-hidden="true">✓</span
                        >{/if}</button
                    >{/each}{:else}<div class="composer-menu-title">
                    {zh ? '由宿主管理权限' : 'Permissions managed by host'}
                  </div>{/if}
              </div>
              {#if state.pendingPermissions.length}<button
                  class="menu-item"
                  type="button"
                  on:click={() => {
                    composerPanel = null;
                    messagesElement?.parentElement
                      ?.querySelector('.permission-shelf')
                      ?.scrollIntoView?.({ block: 'nearest' });
                  }}
                  >{zh ? '查看待批准请求' : 'View pending approvals'} · {state
                    .pendingPermissions.length}</button
                >{/if}
            {/if}
          </aside>{/if}
      </div>
    </footer>
  </section>
</div>
