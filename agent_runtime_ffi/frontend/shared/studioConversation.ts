import {
  AgentRuntimeConversationElement,
  HttpSseConversationTransport,
  type ConversationTransport,
} from '../../../ai-conversation-ui/src/index.js';

export type StudioConversationOptions = {
  token: string;
  locale?: string;
  theme?: 'studio' | 'paper' | 'soft' | 'midnight' | 'neutral' | 'blue' | 'green' | 'amber';
  disclaimer?: string;
  beforeSend?: () => void | Promise<void>;
  onError?: (message: string) => void;
};

export function mountStudioConversation(
  host: HTMLElement,
  options: StudioConversationOptions,
): () => void {
  const base = new HttpSseConversationTransport({
    query: { token: options.token },
    createSendBody: (request) => ({
      message: request.content,
      conversation_id: request.conversationId,
      client_message_id: request.clientMessageId,
      metadata: request.metadata,
    }),
  });
  const transport: ConversationTransport = {
    contract: base.contract,
    id: base.id,
    connect: base.connect.bind(base),
    send: async (request) => {
      await options.beforeSend?.();
      return base.send(request);
    },
    pause: base.pause.bind(base),
    close: base.close.bind(base),
    resolveToolPermission: base.resolveToolPermission.bind(base),
  };
  const element = new AgentRuntimeConversationElement();
  element.transport = transport;
  element.locale = options.locale ?? 'en-US';
  element.theme = options.theme ?? 'studio';
  element.disclaimer = options.disclaimer ?? '';
  element.style.height = '100%';

  const handleError = (event: Event) => {
    const detail = (event as CustomEvent<{ message?: string }>).detail;
    options.onError?.(detail?.message ?? 'Conversation error');
  };
  element.addEventListener('agent-conversation-error', handleError);
  host.replaceChildren(element);
  void element.connect().catch((error) => {
    options.onError?.(error instanceof Error ? error.message : String(error));
  });

  return () => {
    element.removeEventListener('agent-conversation-error', handleError);
    element.disconnect();
    element.remove();
  };
}
