import { mount, unmount, type ComponentProps } from 'svelte';
import Conversation from './lib/Conversation.svelte';
import type { ConversationPublicEventMap } from '@agent-runtime/conversation-element/host';
import 'katex/dist/katex.min.css';

export { Conversation };
export { default as RichContent } from './lib/RichContent.svelte';
export { default as ProviderEditor } from './lib/ProviderEditor.svelte';
export { default as PlanCard } from './lib/PlanCard.svelte';
export * from '@agent-runtime/conversation-element/host';
export * from '@agent-runtime/conversation-element/protocol';
export * from '@agent-runtime/conversation-element/transport';

export type ConversationProps = ComponentProps<typeof Conversation>;
export type ConversationEvents = Partial<{
  [K in keyof ConversationPublicEventMap]: (
    event: CustomEvent<ConversationPublicEventMap[K]>,
  ) => void;
}>;

/** A compiled, framework-independent entry: callers need no Svelte compiler. */
export function mountConversation(
  target: HTMLElement,
  props: ConversationProps = {},
  events: ConversationEvents = {},
) {
  const api = mount(Conversation, { target, props, events });
  let destroyed = false;
  return {
    api,
    async destroy() {
      if (destroyed) return;
      destroyed = true;
      await unmount(api);
    },
  };
}
