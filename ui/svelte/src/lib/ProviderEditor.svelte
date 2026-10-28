<script lang="ts">
  import {
    bindProviderModal,
    providerPresetModels,
    providerEditorValidation,
    providerTabKey,
  } from '@agent-runtime/conversation-element/content';
  import { createEventDispatcher, onMount } from 'svelte';
  import type {
    BuiltinProviderCatalog,
    ConversationProviderController,
    ProviderDefinitionsResult,
    ModelReasoningConfig,
  } from '@agent-runtime/conversation-element/host';
  export let controller: ConversationProviderController;
  export let definitions: ProviderDefinitionsResult;
  export let providerUid: number | null = null;
  export let locale = 'en-US';
  $: zh = locale.startsWith('zh');
  let catalog: BuiltinProviderCatalog = {};
  const provider = definitions.providers?.find((p) => p.uid === providerUid);
  let presetId = '';
  const datalistId = `provider-model-presets-${crypto.randomUUID()}`;
  let name = provider?.name ?? '';
  let type =
    provider?.type ??
    provider?.builtin_type ??
    provider?.builtinType ??
    'openai';
  let baseUrl = provider?.base_url ?? provider?.baseUrl ?? '';
  let paradigm =
    provider?.api_paradigm ??
    provider?.apiParadigm ??
    'openai_chat_completions';
  let key = '';
  let error = '';
  let saving = false;
  let mode: 'manual' | 'import' = 'manual';
  let importJson = '';
  let fileInput: HTMLInputElement;
  let form: HTMLFormElement;
  let alive = true;
  function modal(node: HTMLElement) {
    const destroy = bindProviderModal(node, {
      close: () => emit('cancel'),
      busy: () => saving,
    });
    return { destroy };
  }
  let models =
    definitions.models
      ?.filter((m) => (m.provider_uid ?? m.providerUid) === providerUid)
      .map((m) => ({
        uid: m.uid,
        name:
          m.model_id ??
          m.modelId ??
          m.model_name ??
          m.modelName ??
          m.name ??
          '',
        context: m.context_window ?? m.contextWindow ?? 128000,
        reasoning: { ...m.reasoning } as ModelReasoningConfig,
      })) ?? [];
  if (!models.length)
    models = [{ uid: 0, name: '', context: 128000, reasoning: {} }];
  const emit = createEventDispatcher<{ saved: void; cancel: void }>();
  onMount(() => {
    void controller
      .getBuiltinProviderCatalog?.()
      .then((value) => {
        if (alive) catalog = value;
      })
      .catch((cause) => {
        if (alive) error = String(cause);
      });
    return () => {
      alive = false;
      key = '';
    };
  });
  function useBuiltin(id: string) {
    const preset = catalog.providers?.find((p) => p.id === id);
    if (!preset) return;
    type = preset.id;
    name = preset.name;
    baseUrl = preset.defaultBaseUrl ?? preset.default_base_url ?? '';
    paradigm =
      (preset.apiFormat ?? preset.api_format) === 'anthropic'
        ? 'anthropic_messages'
        : 'openai_chat_completions';
    const defaults = providerPresetModels(catalog, id);
    models = defaults.length
      ? defaults.map((m) => ({
          uid: 0,
          name: m.id,
          context: m.contextWindow ?? m.context_window ?? 128000,
          reasoning: {},
        }))
      : [{ uid: 0, name: '', context: 128000, reasoning: {} }];
  }
  function setReasoning(
    index: number,
    field: 'effort' | 'summary' | 'thinking',
    value: string,
  ) {
    models = models.map((model, i) =>
      i === index
        ? {
            ...model,
            reasoning: { ...model.reasoning, [field]: value || undefined },
          }
        : model,
    );
  }
  function reasoning(
    config: ModelReasoningConfig,
  ): ModelReasoningConfig | undefined {
    const value: ModelReasoningConfig = { effort: config.effort };
    if (paradigm === 'openai_responses') value.summary = config.summary;
    if (paradigm === 'anthropic_messages') {
      if (value.effort === 'none' || value.effort === 'minimal')
        value.effort = undefined;
      value.thinking = config.thinking;
      if (value.thinking === 'enabled')
        value.budget_tokens = config.budget_tokens;
    }
    return Object.values(value).some((v) => v !== undefined)
      ? value
      : undefined;
  }
  async function save() {
    if (saving) return;
    saving = true;
    error = '';
    try {
      if (mode === 'import') {
        try {
          JSON.parse(importJson);
        } catch {
          throw new Error(
            zh
              ? 'JSON 格式无效，请检查后重试。'
              : 'Invalid JSON. Check the configuration and retry.',
          );
        }
        const result = await controller.configureProviders({
          input: importJson,
          source: 'json',
        });
        if (!result.accepted)
          throw new Error(result.rejectReason ?? 'Configuration rejected.');
        if (alive) emit('saved');
        return;
      }
      const invalid = providerEditorValidation(name, baseUrl, models, zh);
      if (invalid) throw new Error(invalid);
      const id =
        providerUid ??
        Math.max(0, ...(definitions.providers ?? []).map((p) => p.uid)) + 1;
      let nextUid =
        Math.max(1000, ...(definitions.models ?? []).map((m) => m.uid)) + 1;
      const existing = (definitions.providers ?? [])
        .filter((p) => p.uid !== id)
        .map((p) => ({
          id: p.uid,
          name: p.name ?? `Provider ${p.uid}`,
          type:
            p.type ??
            p.builtin_type ??
            p.builtinType ??
            ((p.api_paradigm ?? p.apiParadigm) === 'anthropic_messages'
              ? 'claude'
              : 'openai'),
          api_key: '',
          base_url: p.base_url ?? p.baseUrl ?? '',
          api_paradigm:
            p.api_paradigm ?? p.apiParadigm ?? 'openai_chat_completions',
          prompt_cache_control: p.prompt_cache_control ?? p.promptCacheControl,
          enabled_models: (definitions.models ?? [])
            .filter((m) => (m.provider_uid ?? m.providerUid) === p.uid)
            .map((m) => ({
              uid: m.uid,
              model_id:
                m.model_id ??
                m.modelId ??
                m.model_name ??
                m.modelName ??
                m.name ??
                '',
              max_context_tokens: m.context_window ?? m.contextWindow ?? 128000,
              reasoning: m.reasoning,
            }))
            .filter((m) => m.model_id),
        }))
        .filter((p) => p.enabled_models.length > 0);
      const enabled = models.map((m) => ({
        uid: m.uid || nextUid++,
        model_id: m.name.trim(),
        max_context_tokens: m.context,
        reasoning: reasoning(m.reasoning),
      }));
      const providers = [
        ...existing,
        {
          id,
          name: name.trim(),
          type,
          api_key: key,
          base_url: baseUrl.trim(),
          api_paradigm: paradigm,
          prompt_cache_control:
            provider?.prompt_cache_control ?? provider?.promptCacheControl,
          enabled_models: enabled,
        },
      ];
      const allIds = new Set(
        providers.flatMap((p) => p.enabled_models.map((m) => m.uid)),
      );
      const current =
        definitions.current_model_uid ?? definitions.currentModelUid;
      const input = JSON.stringify({
        schema: 'agent-runtime-llm-registration/v1',
        id: 'conversation-provider-editor',
        providers,
        current_model_uid:
          current && allIds.has(current) ? current : enabled[0].uid,
      });
      const result = await controller.configureProviders({
        input,
        source: 'json',
      });
      if (!result.accepted)
        throw new Error(result.rejectReason ?? 'Configuration rejected.');
      key = '';
      if (alive) emit('saved');
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      saving = false;
    }
  }
  async function readFile(event: Event) {
    const input = event.currentTarget as HTMLInputElement,
      file = input.files?.[0];
    input.value = '';
    if (!file || saving) return;
    try {
      const text = await file.text();
      if (alive) {
        importJson = text;
        error = '';
      }
    } catch (cause) {
      if (alive) error = String(cause);
    }
  }
</script>

<div
  class="provider-editor"
  use:modal
  role="dialog"
  aria-modal="true"
  aria-label={zh ? '厂商配置' : 'Provider configuration'}
  aria-busy={saving}
  tabindex="-1"
>
  <form
    class="provider-editor-form"
    bind:this={form}
    on:submit|preventDefault={() => void save()}
  >
    <header class="provider-editor-header">
      <h2>
        {providerUid === null
          ? zh
            ? '添加模型厂商'
            : 'Add model provider'
          : zh
            ? '编辑模型厂商'
            : 'Edit model provider'}
      </h2>
      <button
        class="provider-editor-close"
        type="button"
        aria-label={zh ? '关闭厂商配置' : 'Close provider settings'}
        disabled={saving}
        on:click={() => emit('cancel')}>×</button
      >
    </header>
    <div
      class="provider-editor-tabs"
      role="tablist"
      aria-label={zh ? '配置方式' : 'Configuration mode'}
    >
      {#each ['manual', 'import'] as tab}<button
          class="provider-editor-tab"
          type="button"
          role="tab"
          tabindex={mode === tab ? 0 : -1}
          on:keydown={providerTabKey}
          aria-selected={mode === tab}
          disabled={saving}
          on:click={() => {
            mode = tab as 'manual' | 'import';
            error = '';
          }}
          >{tab === 'manual'
            ? zh
              ? '手动配置'
              : 'Configure'
            : zh
              ? '导入 JSON'
              : 'Import JSON'}</button
        >{/each}
    </div>
    <fieldset
      class="provider-editor-body"
      disabled={saving || mode !== 'manual'}
      hidden={mode !== 'manual'}
    >
      <div class="provider-editor-grid">
        <label class="provider-editor-field wide"
          ><span>{zh ? '内置预设' : 'Built-in preset'}</span><select
            name="builtin-provider"
            bind:value={presetId}
            on:change={(event) => useBuiltin(event.currentTarget.value)}
            ><option value="">{zh ? '自定义厂商' : 'Custom provider'}</option
            >{#each catalog.providers ?? [] as preset}<option value={preset.id}
                >{preset.name}</option
              >{/each}</select
          ></label
        >
        <label class="provider-editor-field"
          ><span>{zh ? '厂商名称' : 'Provider name'}</span><input
            name="provider-name"
            required
            autocomplete="off"
            placeholder="OpenAI Compatible"
            bind:value={name}
          /></label
        >
        <label class="provider-editor-field"
          ><span>{zh ? '接口协议' : 'API protocol'}</span><select
            name="api-paradigm"
            bind:value={paradigm}
            ><option value="openai_chat_completions"
              >OpenAI Chat Completions</option
            ><option value="openai_responses">OpenAI Responses</option><option
              value="anthropic_messages">Anthropic Messages</option
            ></select
          ></label
        >
        <label class="provider-editor-field wide"
          ><span>API Endpoint</span><input
            name="base-url"
            required
            type="url"
            autocomplete="url"
            placeholder="https://api.example.com/v1"
            bind:value={baseUrl}
          /></label
        >
        <label class="provider-editor-field wide"
          ><span>API Key</span><input
            name="api-key"
            type="password"
            bind:value={key}
            autocomplete="new-password"
            placeholder={providerUid === null
              ? 'sk-...'
              : zh
                ? '留空保留现有密钥'
                : 'Leave blank to keep the existing key'}
          /></label
        >
        <div class="provider-editor-models">
          <span class="provider-editor-models-title"
            >{zh ? '模型' : 'Models'}</span
          >
          {#each models as model, index}
            <div class="provider-editor-model-row">
              <label class="provider-editor-field"
                ><span>{zh ? '模型 ID' : 'Model ID'}</span><input
                  name={`model-id-${index}`}
                  required
                  autocomplete="off"
                  placeholder="gpt-4.1"
                  list={datalistId}
                  bind:value={model.name}
                  on:change={() => {
                    const preset = catalog.models?.find(
                      (m) => m.id === model.name,
                    );
                    if (preset)
                      model.context =
                        preset.contextWindow ??
                        preset.context_window ??
                        model.context;
                  }}
                /></label
              >
              <label class="provider-editor-field"
                ><span>{zh ? '上下文长度' : 'Context length'}</span><input
                  name={`model-context-${index}`}
                  type="number"
                  min="1"
                  step="1"
                  required
                  bind:value={model.context}
                /></label
              >
              <button
                class="provider-editor-icon-button secondary"
                type="button"
                aria-label={zh ? '删除模型' : 'Remove model'}
                disabled={models.length === 1}
                on:click={() => (models = models.filter((_, i) => i !== index))}
                >×</button
              >
              <div class="provider-reasoning-settings">
                <label class="provider-editor-field"
                  ><span>{zh ? '推理强度' : 'Reasoning effort'}</span><select
                    name={`model-effort-${index}`}
                    aria-label={zh ? '推理强度' : 'Reasoning effort'}
                    value={model.reasoning.effort ?? ''}
                    on:change={(event) =>
                      setReasoning(index, 'effort', event.currentTarget.value)}
                    ><option value=""
                      >{zh ? '服务端默认' : 'Server default'}</option
                    >{#each paradigm === 'anthropic_messages' ? ['low', 'medium', 'high', 'xhigh', 'max'] : ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] as effort}<option
                        value={effort}>{effort}</option
                      >{/each}</select
                  ></label
                >
                {#if paradigm === 'openai_responses'}<label
                    class="provider-editor-field"
                    ><span>{zh ? '思考摘要' : 'Reasoning summary'}</span><select
                      name={`model-summary-${index}`}
                      value={model.reasoning.summary ?? ''}
                      on:change={(event) =>
                        setReasoning(
                          index,
                          'summary',
                          event.currentTarget.value,
                        )}
                      ><option value="">{zh ? '不指定' : 'Unspecified'}</option
                      >{#each ['auto', 'concise', 'detailed'] as summary}<option
                          value={summary}>{summary}</option
                        >{/each}</select
                    ></label
                  >{/if}
                {#if paradigm === 'anthropic_messages'}<label
                    class="provider-editor-field"
                    ><span>{zh ? '思考模式' : 'Thinking mode'}</span><select
                      name={`model-thinking-${index}`}
                      value={model.reasoning.thinking ?? ''}
                      on:change={(event) =>
                        setReasoning(
                          index,
                          'thinking',
                          event.currentTarget.value,
                        )}
                      ><option value="">{zh ? '不指定' : 'Unspecified'}</option
                      ><option value="adaptive"
                        >{zh ? '自适应' : 'Adaptive'}</option
                      ><option value="enabled"
                        >{zh ? '固定预算' : 'Fixed budget'}</option
                      ><option value="disabled"
                        >{zh ? '关闭' : 'Disabled'}</option
                      ></select
                    ></label
                  >
                  {#if model.reasoning.thinking === 'enabled'}<label
                      class="provider-editor-field"
                      ><span
                        >{zh
                          ? '思考预算（tokens）'
                          : 'Thinking budget (tokens)'}</span
                      ><input
                        name={`model-budget-${index}`}
                        type="number"
                        required
                        min="1024"
                        step="1"
                        bind:value={model.reasoning.budget_tokens}
                      /></label
                    >{/if}
                {/if}
              </div>
            </div>
          {/each}
          <datalist id={datalistId}
            >{#each catalog.models ?? [] as preset}<option value={preset.id}
                >{preset.name}</option
              >{/each}</datalist
          >
          <button
            class="provider-editor-add-model secondary"
            type="button"
            on:click={() =>
              (models = [
                ...models,
                { uid: 0, name: '', context: 128000, reasoning: {} },
              ])}>{zh ? '添加模型' : 'Add model'}</button
          >
        </div>
      </div>
    </fieldset>
    <section class="provider-editor-body" hidden={mode !== 'import'}>
      <div class="provider-editor-import">
        <label class="provider-editor-field"
          ><span>{zh ? '厂商配置 JSON' : 'Provider JSON'}</span><textarea
            aria-label="Provider JSON"
            spellcheck="false"
            bind:value={importJson}
            disabled={saving}
            placeholder={zh
              ? '粘贴厂商配置 JSON'
              : 'Paste provider configuration JSON'}
          ></textarea></label
        ><input
          class="provider-file-input"
          bind:this={fileInput}
          type="file"
          accept=".json,application/json"
          aria-label={zh ? '导入配置文件' : 'Import configuration file'}
          on:change={(event) => void readFile(event)}
        /><button
          class="secondary provider-import-file"
          type="button"
          disabled={saving}
          on:click={() => fileInput.click()}
          >{zh ? '选择 JSON 文件' : 'Choose JSON file'}</button
        >
      </div>
    </section>
    {#if error}<div role="alert" class="provider-editor-error">
        {error}
      </div>{/if}
    <div class="provider-editor-actions">
      <button
        class="secondary"
        type="button"
        disabled={saving}
        on:click={() => emit('cancel')}>{zh ? '取消' : 'Cancel'}</button
      ><button type="submit" disabled={saving}
        >{saving
          ? zh
            ? '保存中'
            : 'Saving'
          : mode === 'import'
            ? zh
              ? '导入配置'
              : 'Import configuration'
            : zh
              ? '保存厂商'
              : 'Save provider'}</button
      >
    </div>
  </form>
</div>
