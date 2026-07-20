import {
  PRESENTATION_CONTRACT,
  TRANSPORT_CONTRACT,
  type AgentRuntimeConversationElement,
  type ConversationConnectContext,
  type ConversationConnection,
  type ConversationTransport,
  type ConversationTransportHandlers,
  type ResolveToolPermissionRequest,
  type SendMessageRequest,
  type SendResult,
} from '../src/index.js';

class PlaygroundTransport implements ConversationTransport {
  readonly contract = TRANSPORT_CONTRACT;
  readonly id = 'playground';
  private handlers: ConversationTransportHandlers | null = null;
  private revision = 1;
  private pendingPermissions: Array<Record<string, unknown>> = [];
  private records: Array<Record<string, unknown>> = [{
    record_id: 'welcome',
    role: 'assistant',
    content: [
      '## Runtime conversation',
      '',
      'This exercises **Markdown**, inline math $E = mc^2$, diagrams, tools, and widgets.',
      '',
      '```mermaid',
      'flowchart LR',
      '  Host --> Transport --> Reducer --> Lit',
      '```',
      '',
      '$$\\int_0^1 x^2\\,dx = \\frac{1}{3}$$',
    ].join('\n'),
  }];

  async connect(
    _context: ConversationConnectContext,
    handlers: ConversationTransportHandlers,
  ): Promise<ConversationConnection> {
    this.handlers = handlers;
    handlers.connection('connected');
    handlers.event({
      type: 'conversation-created',
      conversationId: 'playground-conversation',
    });
    this.emitSnapshot(true);
    return {
      conversationId: 'playground-conversation',
      disconnect: () => { this.handlers = null; },
    };
  }

  async send(request: SendMessageRequest): Promise<SendResult> {
    const turn = this.revision;
    this.records.push({
      record_id: `user-${turn}`,
      role: 'user',
      content: request.content,
    });
    this.emitSnapshot(false);
    globalThis.setTimeout(() => {
      const callId = `demo:${turn}:0`;
      this.records.push(
        {
          record_id: `assistant-${turn}`,
          role: 'assistant',
          content: [
            'I checked the request.',
            `[tool:status | call_id="${callId}"]`,
            '',
            'Choose the next action:',
            '[select:single | label="Action" | options="Continue,Review,Stop"]',
          ].join('\n'),
        },
        {
          record_id: `tool-start-${turn}`,
          role: 'gateway_message',
          content: 'Inspecting the fixture',
          metadata: {
            subtype: 'tool_call_started',
            title: 'Inspect fixture',
            extra: { call_id: callId },
          },
        },
        {
          record_id: `tool-permission-${turn}`,
          role: 'gateway_message',
          content: 'Waiting for permission: InspectFixture',
          metadata: {
            subtype: 'tool_call_permission_requested',
            title: 'Inspect fixture',
            extra: { call_id: callId, status: 'waiting_permission' },
          },
        },
      );
      this.pendingPermissions = [{
        conversation_id: 'playground-conversation',
        tool_call_id: callId,
        agent_id: 'playground-agent',
        tool_name: 'InspectFixture',
        display_name: 'Inspect fixture',
        effect: 'controlled_change',
        arguments: { path: 'fixtures/report.md', mode: 'validate' },
        turn_id: turn,
        created_at: new Date().toISOString(),
      }];
      this.emitSnapshot(false);
    }, 450);
    return { accepted: true, commandId: `command-${turn}` };
  }

  async resolveToolPermission(request: ResolveToolPermissionRequest): Promise<SendResult> {
    this.pendingPermissions = this.pendingPermissions.filter(
      (permission) => permission.tool_call_id !== request.toolCallId,
    );
    this.records.push({
      record_id: `tool-decision-${this.revision}`,
      role: 'tool',
      content: request.decision === 'allow'
        ? 'Fixture is valid.'
        : 'Tool execution was denied.',
      metadata: {
        subtype: 'tool_call_finished',
        title: 'Inspect fixture',
        extra: { call_id: request.toolCallId },
      },
    });
    this.emitSnapshot(true);
    return { accepted: true };
  }

  async pause(): Promise<SendResult> {
    return { accepted: true };
  }

  private emitSnapshot(waiting: boolean): void {
    this.handlers?.event({
      type: 'state-snapshot',
      conversationId: 'playground-conversation',
      payload: {
        revision: this.revision++,
        conversation_state: waiting ? 'waiting' : 'thinking',
        ledger_records: this.records as never,
        pending_permissions: this.pendingPermissions as never,
      },
    });
  }
}

const conversation =
  document.querySelector<AgentRuntimeConversationElement>('#conversation')!;
conversation.transport = new PlaygroundTransport();
conversation.capabilities = {
  openLink: async ({ url }) => {
    globalThis.open(url, '_blank', 'noopener,noreferrer');
  },
  pickPath: async () => ({ paths: ['D:/example/input.txt'] }),
};
void conversation.connect();

document.querySelector<HTMLSelectElement>('#scheme')!.addEventListener(
  'change',
  (event) => {
    conversation.colorScheme =
      (event.target as HTMLSelectElement).value as 'light' | 'dark' | 'system';
  },
);
document.querySelector<HTMLSelectElement>('#theme')!.addEventListener(
  'change',
  (event) => {
    conversation.theme = (event.target as HTMLSelectElement).value;
  },
);
document.querySelector('#preset')!.addEventListener('click', () => {
  const id = `preset-${Date.now()}`;
  conversation.insertPresentationItem({
    contract: PRESENTATION_CONTRACT,
    id,
    scope: 'playground',
    kind: 'assistant-markdown',
    anchor: { type: 'tail' },
    content: `### Local preset\n\nThis message exists only in the frontend.\n\nPreset ID: \`${id}\``,
    reveal: 'progressive',
  });
});
document.querySelector('#external')!.addEventListener('click', () => {
  void conversation.send('Analyze this message from an external host action.', {
    source: 'playground.external',
  });
});
