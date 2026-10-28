import '../src/index.js';
import type { AgentRuntimeConversationElement } from '../src/index.js';
import {
  AcceptanceEnvironment,
  mountAcceptanceControls,
} from '../../playground/acceptance.js';
import '../../playground/acceptance.css';
const environment = new AcceptanceEnvironment();
const conversation =
  document.querySelector<AgentRuntimeConversationElement>('#conversation')!;
conversation.transport = environment;
conversation.capabilities = environment.capabilities;
conversation.composerController = environment.composerController;
conversation.providerControls = {
  enabled: true,
  controller: environment.providerController,
};
conversation.persistence = {
  enabled: true,
  controller: environment.persistenceController,
};
conversation.conversationId = 'acceptance-main';
conversation.locale = 'zh-CN';
conversation.colorScheme = 'light';
conversation.theme = 'white';
conversation.disclaimer = '本页使用模拟数据，AI 输出请核对。';
const controls = document.querySelector<HTMLElement>('#acceptance-controls')!;
const dispose = mountAcceptanceControls(
  controls,
  conversation,
  environment,
  'Lit',
);
conversation.addEventListener('agent-conversation-error', (event) =>
  environment.onAudit(`错误：${(event as CustomEvent).detail.message}`),
);
void conversation.connect();
if (import.meta.hot)
  import.meta.hot.dispose(() => {
    dispose();
    conversation.disconnect();
    environment.dispose();
  });
