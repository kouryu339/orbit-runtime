<script lang="ts">
  import Conversation from './lib/Conversation.svelte';
  import { DemoTransport } from './demo-transport';

  const transport = new DemoTransport();
</script>

<main>
  <Conversation
    {transport}
    locale="zh-CN"
    colorScheme="system"
    disclaimer="AI 输出可能包含错误，请核对重要结果。"
  />
</main>
