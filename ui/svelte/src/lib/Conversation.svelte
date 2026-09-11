<script lang="ts">
  import { createEventDispatcher, onMount, tick } from 'svelte';
  import { visiblePresentationItems } from '@agent-runtime/conversation-element/content';
  import {
    conversationReducer,
    createConversationState,
    createPendingUserMessage,
    displayText,
    displayTextWithToolAnchors,
    recordKey,
    summarizePermissionTargets,
    type ConversationAction,
    type ConversationState,
    type LedgerRecord,
    type PendingToolPermission,
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
    SendOptions, CommandResult, ConversationHostCapabilities, ConversationExtension,
    ConversationPresentationItem, PresentationItemPatch, PresetMarkdown,
    ConversationPersistencePolicy, ConversationProviderControlsPolicy,
    ConversationThemeName, ConversationDensity, ConversationPublicEventMap,
  } from '@agent-runtime/conversation-element/host';
  import RichContent from './RichContent.svelte';
  import ProviderEditor from './ProviderEditor.svelte';

  export let transport: ConversationTransport | null = null;
  export let capabilities: ConversationHostCapabilities = {};
  export let extensions: readonly ConversationExtension[] = [];
  export let presentationItems: readonly ConversationPresentationItem[] = [];
  export let persistence: ConversationPersistencePolicy = { enabled: false };
  export let providerControls: ConversationProviderControlsPolicy = { enabled: false };
  export let theme: ConversationThemeName = 'paper';
  export let density: ConversationDensity = 'comfortable';
  export let conversationId: string | null = null;
  export let locale = 'zh-CN';
  export let colorScheme: 'light' | 'dark' | 'system' = 'system';
  $: persistenceController = persistence.enabled ? persistence.controller : null;
  $: providerController = providerControls.enabled ? providerControls.controller : null;
  export let hideToolCalls = false;
  export let disclaimer = '';

  export let state: ConversationState = createConversationState(conversationId);
  const emit = createEventDispatcher<ConversationPublicEventMap>();
  let generation = 0;
  let pausing = false;
  let mounted = false;
  let composerElement: HTMLTextAreaElement;
  let completedReveal = new Set<string>();
  let unsubscribePersistence: (() => void) | undefined;
  let deferred: {content: string; options: SendOptions; resolve: (r: SendResult) => void; cleanup: () => void} | null = null;
  let providerInput = '';
  let providerError = '';
  let editorOpen = false;
  let editorProviderUid: number | null = null;
  let bindingArchiveId: string | undefined;
  let connection: { disconnect(): void | Promise<void> } | null = null;
  let draft = '';
  let messagesElement: HTMLElement;
  let operation = '';
  let permissionError = '';
  let historyOpen = false;
  let providerOpen = false;
  let archives: PersistedConversation[] = [];
  let providers: ProviderDefinitionsResult | null = null;
  let localSequence = 0;
  $: dark = colorScheme === 'dark' ||
    (colorScheme === 'system' && globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches);
  $: displayRecords = buildDisplayRecords(state, presentationItems, completedReveal);
  $: active = state.runtimeState !== 'waiting' || state.awaitingAssistantResponse;
  $: canCompose = state.initialized && state.snapshotReceived && !active && !pausing;
  $: bindPersistence(persistenceController);
  $: latestWidget = [...displayRecords].reverse().find(r => r.role === 'assistant' && /\[(?:select:|input:|confirm)/.test(r.content ?? ''))?.record_id;

  function bindPersistence(controller: ConversationPersistenceController | null) {
    unsubscribePersistence?.();
    unsubscribePersistence = controller?.subscribe?.(() => { void refreshConversationHistory(); });
  }

  function rejectDeferred(reason: string) {
    const pending = deferred; deferred = null;
    pending?.cleanup(); pending?.resolve({accepted: false, rejectReason: reason});
  }

  function dispatch(action: ConversationAction) {
    state = conversationReducer(state, action);
    conversationId = state.conversationId;
    emit('agent-conversation-state-change', { state, reason: action.type });
    if (action.type === 'snapshot' && state.runtimeState === 'waiting') pausing = false;
    if (state.initialized && state.snapshotReceived && !state.awaitingAssistantResponse && state.runtimeState === 'waiting' && !pausing) {
      const pending = deferred; deferred = null;
      if (pending) { pending.cleanup(); void send(pending.content, pending.options).then(pending.resolve); }
    }
    void tick().then(() => {
      if (messagesElement) messagesElement.scrollTop = messagesElement.scrollHeight;
    });
  }

  export async function connect() {
    const epoch = ++generation;
    const previous = connection; connection = null;
    await previous?.disconnect();
    if (epoch !== generation) return;
    rejectDeferred('Conversation connection changed.');
    if (!transport) return;
    pausing = false;
    dispatch({ type: 'reset', conversationId });
    dispatch({ type: 'connection', state: 'connecting' });
    try {
      const connected = await transport.connect(
        { conversationId, locale },
        {
          event: (value) => { if (epoch === generation) handleEvent(value); },
          connection: (value) => { if (epoch === generation) dispatch({ type: 'connection', state: value }); },
          error: (value) => { if (epoch === generation) handleError(value); },
        },
      );
      if (epoch !== generation) { await connected.disconnect(); return; }
      connection = connected;
    } catch (cause) {
      if (epoch !== generation) return;
      rejectDeferred('Connection failed.');
      handleError({
        code: 'connect-failed',
        message: cause instanceof Error ? cause.message : String(cause),
        recoverable: true,
      });
      dispatch({ type: 'connection', state: 'disconnected' });
    }
  }

  export function disconnect() {
    ++generation;
    const previous = connection; connection = null;
    void previous?.disconnect();
    rejectDeferred('Conversation disconnected.');
    dispatch({ type: 'connection', state: 'disconnected' });
  }

  function handleEvent(event: ConversationTransportEvent) {
    if (event.type === 'conversation-created') {
      dispatch({ type: 'conversation-created', conversationId: event.conversationId, eventSeq: event.eventSeq });
    } else if (event.type === 'conversation-closed') {
      dispatch({ type: 'conversation-closed', conversationId: event.conversationId });
    } else if (event.type === 'state-snapshot') {
      if (event.conversationId && state.conversationId && event.conversationId !== state.conversationId) return;
      const first = !state.snapshotReceived;
      dispatch({ type: 'snapshot', payload: event.payload, eventSeq: event.eventSeq });
      if (first && state.conversationId) emit('agent-conversation-ready', {conversationId: state.conversationId, state});
    } else if (event.type === 'pending-user-message') {
      if (event.conversationId !== state.conversationId) return;
      if (state.pendingUserMessages.some((message) => message.id === event.messageId)) return;
      dispatch({
        type: 'local-message-added',
        message: createPendingUserMessage(event.messageId, event.content, event.createdAt),
      });
      dispatch({ type: 'local-message-accepted', id: event.messageId });
    } else if (event.type === 'transport-extension') {
      if (extensions.some(item => item.namespace === event.extension.namespace && item.version === event.extension.version && item.kinds.includes(event.extension.kind))) {
        emit('agent-conversation-extension-action', {extension:event.extension, action:'received'});
      }
    }
  }

  function handleError(error: ConversationTransportError) {
    state = { ...state, lastError: error.message };
    emit('agent-conversation-error', error);
  }

  export async function send(content = draft, options: SendOptions = {}): Promise<SendResult> {
    const value = content.trim();
    if (!value || !transport || options.signal?.aborted) return { accepted: false, rejectReason: 'Message is empty, aborted, or transport unavailable.' };
    if (!state.initialized || !state.snapshotReceived || pausing) {
      if (deferred) return {accepted:false, rejectReason:'A message is already waiting.'};
      return new Promise(resolve => {
        const abort = () => rejectDeferred('Message send aborted.');
        options.signal?.addEventListener('abort', abort, {once:true});
        deferred = {content:value, options, resolve, cleanup:() => options.signal?.removeEventListener('abort', abort)};
      });
    }
    if (!state.conversationId || state.runtimeState !== 'waiting' || state.awaitingAssistantResponse) return {accepted:false, rejectReason:'Conversation is still running.'};
    const targetId = state.conversationId;
    const epoch = generation;
    const id = `svelte-${Date.now()}-${++localSequence}`;
    dispatch({ type: 'local-message-added', message: createPendingUserMessage(id, value) });
    draft = '';
    try {
      const result = await transport.send({
        conversationId: state.conversationId,
        content: value,
        clientMessageId: id,
        signal: options.signal,
        metadata: {...options.metadata, source: options.source},
      });
      if (epoch === generation) dispatch(result.accepted
        ? { type: 'local-message-accepted', id }
        : { type: 'local-message-failed', id, error: result.rejectReason ?? 'Message rejected.' });
      emit('agent-conversation-send', {conversationId: targetId, clientMessageId: id, content:value, source:options.source, result});
      return result;
    } catch (cause) {
      const error = cause instanceof Error ? cause.message : String(cause);
      if (epoch === generation) dispatch({ type: 'local-message-failed', id, error });
      return { accepted: false, rejectReason: error };
    }
  }

  export async function pause(): Promise<CommandResult> {
    if (!state.conversationId || !transport?.pause) return {accepted:false, rejectReason:'Pause is not supported.'};
    pausing = true;
    try {
      const result = await transport.pause({ conversationId: state.conversationId });
      if (!result.accepted) { pausing = false; rejectDeferred(result.rejectReason ?? 'Pause rejected.'); }
      emit('agent-conversation-pause', {conversationId: state.conversationId, result});
      return result;
    } catch (cause) { pausing = false; rejectDeferred('Pause failed.'); throw cause; }
  }

  export async function openConversation(id: string) {
    conversationId = id;
    await connect();
  }

  export async function createConversation() {
    if (persistenceController?.create) {
      const binding = await persistenceController.create();
      bindingArchiveId = binding.archiveId;
      if (persistence.enabled) persistence = {...persistence,binding};
      await openConversation(binding.runtimeConversationId);
      return;
    }
    conversationId = null;
    bindingArchiveId = undefined;
    await connect();
  }

  export async function saveConversation() {
    if (!persistenceController || !state.conversationId || state.runtimeState !== 'waiting') return null;
    operation = 'save';
    try {
      const saved = await persistenceController.save({ archiveId: bindingArchiveId ?? (persistence.enabled ? persistence.binding?.archiveId : undefined), runtimeConversationId: state.conversationId });
      bindingArchiveId = saved.archiveId;
      await refreshHistory();
      return saved;
    } finally { operation = ''; }
  }

  export async function restoreConversation(archiveId: string) {
    if (!persistenceController) return;
    operation = `restore:${archiveId}`;
    try {
      const binding = await persistenceController.restore({ archiveId });
      bindingArchiveId = binding.archiveId;
      await openConversation(binding.runtimeConversationId);
      historyOpen = false;
    } finally { operation = ''; }
  }

  export async function refreshHistory() {
    if (!persistenceController) return;
    archives = (await persistenceController.list({ limit: 50 })).items;
  }

  export async function refreshConversationHistory() { await refreshHistory(); }
  export async function closeConversation() {
    const id = state.conversationId;
    if (id && transport?.close) {
      const result = await transport.close({conversationId:id});
      if (!result.accepted) throw new Error(result.rejectReason ?? 'Close rejected.');
    }
    disconnect();
    dispatch({type:'conversation-closed', conversationId:id ?? undefined});
  }
  export function focusComposer() { composerElement?.focus(); }
  export function insertPresetMarkdown(preset: PresetMarkdown) {
    const id = preset.id ?? `preset-${Date.now()}-${++localSequence}`;
    const last = [...state.records].reverse().find(r => r.role === 'assistant' || r.role === 'user');
    insertPresentationItem({contract:'agent-conversation-presentation/v1', id, kind:'assistant-markdown',
      scope:preset.scope ?? 'preset-markdown', content:preset.markdown, reveal:'progressive',
      anchor:last ? {type:'after-record', recordId:recordKey(last)} : {type:'tail'},
      metadata:{baseDir:preset.baseDir, presetName:preset.name}, createdAt:preset.createdAt});
    return id;
  }
  export function insertPresentationItem(item: ConversationPresentationItem) {
    presentationItems = [...presentationItems.filter(p => p.id !== item.id), item];
  }
  export function updatePresentationItem(id: string, patch: PresentationItemPatch) {
    completedReveal = new Set([...completedReveal].filter(key => key !== `presentation:${id}`));
    presentationItems = presentationItems.map(item => item.id === id ? {...item,...patch} : item);
  }
  export function removePresentationItem(id: string) { presentationItems = presentationItems.filter(item => item.id !== id); }
  export function clearPresentationItems(scope?: string) { presentationItems = scope ? presentationItems.filter(item => item.scope !== scope) : []; }

  export async function refreshProviders() {
    if (!providerController) return;
    providers = await providerController.getProviderDefinitions();
    emit('agent-conversation-provider-action', {action:'definitions-loaded',definitions:providers});
  }

  export async function setConversationModel(modelUid: number) {
    if (!providerController || !state.conversationId) return;
    const target = state.conversationId;
    const epoch = generation;
    operation = 'model';
    try {
      const result = await providerController.setConversationModel({
        conversationId: state.conversationId,
        modelUid,
      });
      if (result.accepted !== false && epoch === generation && target === state.conversationId) {
        const model = providers?.models?.find((item) => item.uid === modelUid);
        dispatch({ type: 'conversation-model-selected', model: modelName(model), modelUid });
        emit('agent-conversation-provider-action', {action:'model-selected',modelUid,result});
      }
    } finally { operation = ''; }
  }

  export async function resolveToolPermission(toolCallId: string, decision: 'allow' | 'deny'): Promise<CommandResult> {
    if (!transport?.resolveToolPermission || !state.conversationId) return {accepted:false, rejectReason:'Permission response unsupported.'};
    operation = toolCallId;
    permissionError = '';
    try {
      const result = await transport.resolveToolPermission({
        conversationId: state.conversationId,
        toolCallId,
        decision,
      });
      if (!result.accepted) permissionError = result.rejectReason ?? 'Request is no longer pending.';
      else state = {...state, pendingPermissions:state.pendingPermissions.filter(p => p.tool_call_id !== toolCallId)};
      emit('agent-conversation-tool-permission', {conversationId:state.conversationId!, toolCallId, decision, result});
      return result;
    } catch (cause) {
      permissionError = cause instanceof Error ? cause.message : String(cause);
      return {accepted:false, rejectReason:permissionError};
    } finally { operation = ''; }
  }

  async function configureProviders() {
    if (!providerController || !providerInput.trim()) return;
    providerError = ''; operation = 'providers';
    try {
      JSON.parse(providerInput);
      const result = await providerController.configureProviders({input:providerInput, source:'json'});
      if (!result.accepted) throw new Error(result.rejectReason ?? 'Provider configuration rejected.');
      providerInput = ''; await refreshProviders();
    } catch (cause) { providerError = cause instanceof Error ? cause.message : String(cause); }
    finally { operation = ''; }
  }

  function modelName(model: ProviderModelDefinition | undefined): string {
    return model?.model_name ?? model?.modelName ?? model?.model_id ?? model?.modelId ?? model?.name ?? '';
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void send();
    }
  }

  onMount(() => {
    mounted = true;
    void connect();
    return () => { mounted = false; disconnect(); unsubscribePersistence?.(); };
  });

  function buildDisplayRecords(value: ConversationState, presentations: readonly ConversationPresentationItem[], completed: Set<string>): (LedgerRecord & {presentation?: ConversationPresentationItem})[] {
    const records = value.records.filter((record) => record.role === 'user' || record.role === 'assistant');
    const items: (LedgerRecord & {presentation?: ConversationPresentationItem})[] = [...records];
    const offsets = new Map<string, number>();
    for (const item of visiblePresentationItems(presentations, completed)) {
      const record = {record_id:`presentation:${item.id}`, role:'assistant' as const, content:item.content ?? '', presentation:item};
      const anchor = item.anchor;
      const key = anchor.type === 'head' || anchor.type === 'tail' ? anchor.type : `${anchor.type}:${anchor.recordId}`;
      const offset = offsets.get(key) ?? 0;
      const index = 'recordId' in anchor ? items.findIndex(r => recordKey(r) === anchor.recordId) : -1;
      if (anchor.type === 'head') items.splice(offset,0,record);
      else if (index < 0) items.push(record);
      else items.splice(anchor.type === 'before-record' ? index : index + 1 + offset,0,record);
      offsets.set(key,offset + 1);
    }
    const pending = value.pendingUserMessages.map((message) => ({
      record_id: message.id,
      role: 'user' as const,
      content: message.state === 'failed' ? `${message.content}\n${message.error ?? ''}` : message.content,
    }));
    const stream = value.assistantStream;
    const streamRecord = stream && (stream.content || stream.provisional_tool_calls?.length)
      ? [{
          record_id: `stream:${stream.agent_id}:${stream.turn_id}:${stream.attempt}`,
          role: 'assistant' as const,
          content: [
            stream.content,
            ...(stream.provisional_tool_calls ?? []).map((call) =>
              `[tool:status | call_id="${call.call_id || call.key}"]`),
          ].filter(Boolean).join('\n'),
        }]
      : [];
    return [...items, ...pending, ...streamRecord];
  }
</script>

<section class:dark class="conversation-shell" data-theme={theme} data-density={density} lang={locale}>
  <header class="topbar">
    <div class="connection"><span class:online={state.connection === 'connected'}></span>{state.connection}</div>
    <div class="top-actions">
      {#if providerController && (!providerControls.enabled || providerControls.showModelSwitcher !== false)}
        <button class="quiet" on:click={() => { providerOpen = !providerOpen; if (providerOpen) void refreshProviders(); }}>
          {state.model || '选择模型'}
        </button>
      {/if}
      {#if persistenceController && (!persistence.enabled || persistence.showHistory !== false)}
        <button class="quiet" on:click={() => { historyOpen = !historyOpen; if (historyOpen) void refreshHistory(); }}>历史</button>
        <button class="quiet" disabled={operation === 'save'} on:click={() => void saveConversation()}>保存</button>
      {/if}
      <button class="quiet" on:click={() => void createConversation()}>新会话</button>
    </div>
  </header>

  {#if providerOpen && providers}
    <aside class="panel model-panel">
      <strong>当前会话模型</strong>
      <select value={state.modelUid ?? ''} disabled={operation === 'model'} on:change={(event) => void setConversationModel(Number(event.currentTarget.value))}>
        <option value="" disabled>选择模型</option>
        {#each providers.models ?? [] as model}<option value={model.uid}>{modelName(model)}</option>{/each}
      </select>
      <div class="provider-list">
        {#each providers.providers ?? [] as provider}
          <div><span>{provider.name ?? provider.uid}</span><button on:click={() => {editorProviderUid=provider.uid;editorOpen=true;}}>编辑</button></div>
        {/each}
        <button on:click={() => {editorProviderUid=null;editorOpen=true;}}>添加厂商</button>
      </div>
      {#if editorOpen && providerController}
        {#key editorProviderUid}<ProviderEditor controller={providerController} definitions={providers} providerUid={editorProviderUid}
          on:cancel={() => editorOpen=false} on:saved={() => {editorOpen=false;void refreshProviders();}} />{/key}
      {/if}
      {#if providerControls.enabled && providerControls.showImport !== false}
        <label>厂商配置 JSON<textarea aria-label="厂商配置 JSON" bind:value={providerInput} placeholder="粘贴新增或更新的厂商配置"></textarea></label>
        <button disabled={operation === 'providers' || !providerInput.trim()} on:click={() => void configureProviders()}>导入厂商配置</button>
        {#if providerError}<p role="alert" class="error">{providerError}</p>{/if}
      {/if}
      <button on:click={() => void refreshProviders()}>刷新</button>
    </aside>
  {/if}

  {#if historyOpen}
    <aside class="panel history-panel">
      <strong>已保存会话</strong>
      {#if !archives.length}<p>暂无会话</p>{/if}
      {#each archives as archive}
        <button disabled={operation === `restore:${archive.archiveId}`} on:click={() => void restoreConversation(archive.archiveId)}>
          <span>{archive.title || '未命名会话'}</span><small>{archive.preview || archive.updatedAt || ''}</small>
        </button>
      {/each}
    </aside>
  {/if}

  <main class="messages" bind:this={messagesElement} aria-live="polite">
    {#if !displayRecords.length}<div class="empty">开始一段对话</div>{/if}
    {#each displayRecords as record (recordKey(record))}
      <article class:assistant={record.role === 'assistant'} class:user={record.role === 'user'} class="message">
        {#if record.role === 'assistant'}
          <RichContent
            content={displayTextWithToolAnchors(record)}
            toolCalls={state.toolCalls}
            {dark}
            {hideToolCalls}
            {capabilities}
            {locale}
            reveal={record.presentation?.reveal === 'progressive' && !completedReveal.has(String(record.record_id))}
            widgetsExpired={record.record_id !== latestWidget}
            baseDir={String(record.presentation?.metadata?.baseDir ?? '')}
            on:complete={() => { completedReveal = new Set([...completedReveal, String(record.record_id)]); }}
            on:submit={(event) => void send(event.detail.content, {source:'widget'})}
          />
        {:else}<div class="user-bubble">{displayText(record)}</div>{/if}
      </article>
    {/each}
    {#if active && (!state.assistantStream?.content || state.runtimeState === 'executing')}
      <div class="activity"><span></span>{state.runtimeState === 'executing' ? '工具执行中' : '加载中'}</div>
    {/if}
    {#if state.lastError}<div class="error" role="alert">{state.lastError}</div>{/if}
  </main>

  {#if state.pendingPermissions.length}
    <section class="approval" aria-label="工具批准请求">
      <strong>{state.pendingPermissions.length === 1 ? '等待批准' : `${state.pendingPermissions.length} 个请求等待批准`}</strong>
      {#each state.pendingPermissions as permission}
        <div class="approval-row">
          <div>
            <b>{permission.display_name || permission.tool_name}</b>
            <small>{permission.effect} · {permission.tool_name}</small>
            {#if summarizePermissionTargets(permission).length}
              <dl>
                {#each summarizePermissionTargets(permission) as target}<dt>{target.label}</dt><dd>{target.value}</dd>{/each}
              </dl>
            {/if}
            {#if permissionError}<p class="error">{permissionError}</p>{/if}
          </div>
          <div class="approval-actions">
            <button disabled={operation === permission.tool_call_id} on:click={() => void resolveToolPermission(permission.tool_call_id, 'deny')}>拒绝</button>
            <button class="primary" disabled={operation === permission.tool_call_id} on:click={() => void resolveToolPermission(permission.tool_call_id, 'allow')}>允许</button>
          </div>
        </div>
      {/each}
    </section>
  {/if}

  <footer class="composer">
    <textarea bind:this={composerElement} bind:value={draft} on:keydown={onKeydown} disabled={!canCompose} placeholder="向 Agent 发送消息"></textarea>
    {#if active}<button class="pause" on:click={() => void pause()} aria-label="暂停">Ⅱ</button>{/if}
    <button class="send" disabled={!draft.trim() || !canCompose} on:click={() => void send()} aria-label="发送">↗</button>
  </footer>
  {#if disclaimer && state.runtimeState === 'waiting'}<div class="disclaimer">{disclaimer}</div>{/if}
</section>
