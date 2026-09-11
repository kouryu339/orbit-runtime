import { TRANSPORT_CONTRACT } from '@agent-runtime/conversation-element/host';
import type {
  ConversationConnection,
  ConversationTransport,
  ConversationTransportHandlers,
  ResolveToolPermissionRequest,
  SendMessageRequest,
  SendResult,
} from '@agent-runtime/conversation-element/host';

export class DemoTransport implements ConversationTransport {
  readonly contract = TRANSPORT_CONTRACT;
  readonly id = 'svelte-demo';
  private handlers: ConversationTransportHandlers | null = null;
  private revision = 1;
  private records: Array<Record<string, unknown>> = [];
  private permissions: Array<Record<string, unknown>> = [];

  async connect(_context: unknown, handlers: ConversationTransportHandlers): Promise<ConversationConnection> {
    this.handlers = handlers;
    handlers.connection('connected');
    handlers.event({ type: 'conversation-created', conversationId: 'svelte-demo' });
    this.snapshot('waiting');
    return { conversationId: 'svelte-demo', disconnect: () => { this.handlers = null; } };
  }

  async send(request: SendMessageRequest): Promise<SendResult> {
    const turn = this.revision;
    const callId = `demo-call-${turn}`;
    this.records.push({ record_id: `u-${turn}`, role: 'user', content: request.content });
    this.snapshot('thinking');
    setTimeout(() => {
      this.records.push({
        record_id: `a-${turn}`,
        role: 'assistant',
        content: '页面已打开，现在获取页面快照查看其结构和登录状态。',
        metadata: { extra: {
          tool_call_ids: [callId],
          tool_calls: [{ id: callId, function: { name: 'GetSnapshot' } }],
        } },
      }, {
        record_id: `t-${turn}`,
        role: 'gateway_message',
        content: '正在执行 GetSnapshot',
        metadata: { subtype: 'tool_call_started', tool_name: 'GetSnapshot', extra: { call_id: callId } },
      });
      this.snapshot('executing');
    }, 350);
    setTimeout(() => {
      this.permissions = [{
        conversation_id: 'svelte-demo', tool_call_id: callId, agent_id: 'boss',
        tool_name: 'GetSnapshot', display_name: '读取页面快照', effect: 'read_only',
        arguments: { page_id: 'page-7', token: 'do-not-render', content: { raw: true } },
      }];
      this.snapshot('executing');
    }, 700);
    return { accepted: true };
  }

  async pause(): Promise<SendResult> { this.snapshot('waiting'); return { accepted: true }; }

  async resolveToolPermission(request: ResolveToolPermissionRequest): Promise<SendResult> {
    this.permissions = [];
    this.records.push({
      record_id: `done-${this.revision}`,
      role: 'tool',
      content: request.decision === 'allow' ? '快照获取完成' : '已拒绝',
      metadata: { subtype: 'tool_call_finished', tool_name: 'GetSnapshot', extra: { call_id: request.toolCallId } },
    });
    this.snapshot('waiting');
    return { accepted: true };
  }

  private snapshot(state: 'waiting' | 'thinking' | 'executing') {
    this.handlers?.event({
      type: 'state-snapshot', conversationId: 'svelte-demo',
      payload: {
        revision: this.revision++, conversation_state: state,
        ledger_records: this.records as never,
        pending_permissions: this.permissions as never,
      },
    });
  }
}
