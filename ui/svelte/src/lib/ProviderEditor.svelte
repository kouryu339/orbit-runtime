<script lang="ts">
  import { createEventDispatcher, onMount } from 'svelte';
  import type { BuiltinProviderCatalog, ConversationProviderController, ProviderDefinitionsResult } from '@agent-runtime/conversation-element/host';
  export let controller: ConversationProviderController;
  export let definitions: ProviderDefinitionsResult;
  export let providerUid: number | null = null;
  let catalog: BuiltinProviderCatalog = {};
  const provider = definitions.providers?.find(p => p.uid === providerUid);
  let name = provider?.name ?? '';
  let type = provider?.type ?? provider?.builtin_type ?? 'openai';
  let baseUrl = provider?.base_url ?? provider?.baseUrl ?? '';
  let paradigm = provider?.api_paradigm ?? provider?.apiParadigm ?? 'openai_chat_completions';
  let key = '';
  let error = '';
  let saving = false;
  let models = definitions.models?.filter(m => (m.provider_uid ?? m.providerUid) === providerUid)
    .map(m => ({uid:m.uid, name:m.model_id ?? m.modelId ?? m.model_name ?? m.modelName ?? m.name ?? '', context:m.context_window ?? m.contextWindow ?? 128000})) ?? [];
  if (!models.length) models = [{uid:0,name:'',context:128000}];
  const emit = createEventDispatcher<{saved:void; cancel:void}>();
  onMount(() => { void controller.getBuiltinProviderCatalog?.().then(value => catalog = value).catch(cause => error = String(cause)); });
  function useBuiltin(id: string) {
    const preset = catalog.providers?.find(p => p.id === id);
    if (!preset) return;
    type = preset.id; name = preset.name;
    baseUrl = preset.defaultBaseUrl ?? preset.default_base_url ?? '';
    paradigm = preset.apiFormat ?? preset.api_format ?? 'openai_chat_completions';
  }
  async function save() {
    saving = true; error = '';
    try {
      if (!name.trim() || !baseUrl.trim() || models.some(m => !m.name.trim() || m.context <= 0)) throw new Error('请填写厂商、地址和完整模型配置。');
      const id = providerUid ?? Math.max(0,...(definitions.providers ?? []).map(p=>p.uid))+1;
      let nextUid = Math.max(1000,...(definitions.models ?? []).map(m=>m.uid))+1;
      const existing = (definitions.providers ?? []).filter(p => p.uid !== id).map(p => ({
        id:p.uid, name:p.name, type:p.type ?? p.builtin_type ?? p.builtinType ?? 'openai', api_key:'',
        base_url:p.base_url ?? p.baseUrl, api_paradigm:p.api_paradigm ?? p.apiParadigm ?? 'openai_chat_completions',
        prompt_cache_control:p.prompt_cache_control ?? p.promptCacheControl,
        enabled_models:(definitions.models ?? []).filter(m => (m.provider_uid ?? m.providerUid) === p.uid)
          .map(m => ({uid:m.uid,model_id:m.model_id ?? m.modelId ?? m.model_name ?? m.modelName ?? m.name,max_context_tokens:m.context_window ?? m.contextWindow ?? 128000})),
      }));
      const enabled = models.map(m => ({uid:m.uid || nextUid++,model_id:m.name.trim(),max_context_tokens:m.context}));
      const providers = [...existing,{id,name:name.trim(),type,api_key:key,base_url:baseUrl.trim(),api_paradigm:paradigm,
        prompt_cache_control:provider?.prompt_cache_control ?? provider?.promptCacheControl,enabled_models:enabled}];
      const allIds = new Set(providers.flatMap(p=>p.enabled_models.map(m=>m.uid)));
      const current = definitions.current_model_uid ?? definitions.currentModelUid;
      const input = JSON.stringify({schema:'agent-runtime-llm-registration/v1',id:'conversation-provider-editor',providers,
        current_model_uid:current && allIds.has(current) ? current : enabled[0].uid});
      const result = await controller.configureProviders({input,source:'json'});
      if (!result.accepted) throw new Error(result.rejectReason ?? 'Configuration rejected.');
      key = ''; emit('saved');
    } catch(cause) {error = cause instanceof Error ? cause.message : String(cause);}
    finally {saving = false;}
  }
</script>

<form class="provider-editor" on:submit|preventDefault={() => void save()}>
  <strong>{providerUid === null ? '添加厂商' : '编辑厂商'}</strong>
  {#if catalog.providers?.length}
    <label>预设<select on:change={event => useBuiltin(event.currentTarget.value)}><option value="">自定义</option>{#each catalog.providers as preset}<option value={preset.id}>{preset.name}</option>{/each}</select></label>
  {/if}
  <label>名称<input required bind:value={name} /></label>
  <label>API 地址<input required type="url" bind:value={baseUrl} /></label>
  <label>API 范式<select bind:value={paradigm}><option value="openai_chat_completions">OpenAI Chat Completions</option><option value="openai_responses">OpenAI Responses</option><option value="anthropic_messages">Anthropic Messages</option></select></label>
  <label>API Key<input type="password" bind:value={key} autocomplete="new-password" placeholder={providerUid === null ? '' : '留空保留现有密钥'} /></label>
  {#each models as model,index}
    <div class="provider-model-row">
      <label>模型 ID<input required list="provider-model-presets" bind:value={model.name} /></label>
      <label>上下文长度<input type="number" min="1" required bind:value={model.context} /></label>
      <button type="button" disabled={models.length === 1} on:click={() => models = models.filter((_,i)=>i!==index)}>移除</button>
    </div>
  {/each}
  <datalist id="provider-model-presets">{#each catalog.models ?? [] as preset}<option value={preset.id}>{preset.name}</option>{/each}</datalist>
  <button type="button" on:click={() => models = [...models,{uid:0,name:'',context:128000}]}>添加模型</button>
  {#if error}<p role="alert" class="error">{error}</p>{/if}
  <div><button type="button" on:click={() => emit('cancel')}>取消</button> <button class="primary" disabled={saving}>保存厂商</button></div>
</form>
