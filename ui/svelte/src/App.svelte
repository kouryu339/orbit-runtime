<script lang="ts">
  import { onMount } from 'svelte';
  import Conversation from './lib/Conversation.svelte';
  import {
    AcceptanceEnvironment,
    mountAcceptanceControls,
    type AcceptanceApi,
  } from '../../playground/acceptance.js';
  import '../../playground/acceptance.css';
  const environment = new AcceptanceEnvironment();
  const providerControls = {
    enabled: true as const,
    controller: environment.providerController,
  };
  const persistence = {
    enabled: true as const,
    controller: environment.persistenceController,
  };
  let conversation: AcceptanceApi;
  let controls: HTMLElement;
  let theme = 'white';
  let colorScheme: 'light' | 'dark' | 'system' = 'light';
  let density: 'compact' | 'comfortable' = 'comfortable';
  let locale = 'zh-CN';
  let hideToolCalls = false;
  let progressiveReveal = true;
  onMount(() => {
    const dispose = mountAcceptanceControls(
      controls,
      conversation,
      environment,
      'Svelte',
    );
    return () => {
      dispose();
      environment.dispose();
    };
  });
</script>

<svelte:head><title>Svelte · 前端验收</title></svelte:head>
<main class="acceptance-page">
  <aside
    class="acceptance-controls"
    bind:this={controls}
    aria-label="验收控制"
  ></aside>
  <div class="acceptance-conversation">
    <Conversation
      bind:this={conversation}
      transport={environment}
      capabilities={environment.capabilities}
      composerController={environment.composerController}
      {providerControls}
      {persistence}
      conversationId="acceptance-main"
      bind:theme
      bind:colorScheme
      bind:density
      bind:locale
      bind:hideToolCalls
      bind:progressiveReveal
      disclaimer="本页使用模拟数据，AI 输出请核对。"
      on:agent-conversation-error={(event) =>
        environment.onAudit(`错误：${event.detail.message}`)}
    />
  </div>
</main>
