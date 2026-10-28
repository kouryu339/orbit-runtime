import {
  TRANSPORT_CONTRACT,
  PERSISTENCE_CONTRACT,
  type AgentRuntimeConversationPublicApi,
  type ConversationTransport,
  type ConversationTransportHandlers,
  type SendMessageRequest,
  type ConversationPersistenceController,
  type ConversationProviderController,
  type ConversationHostCapabilities,
  type ConversationComposerController,
  type ReasoningEffort,
  type ProviderDefinitionsResult,
  type PersistedConversation,
  type ConversationPersistenceEvent,
} from '../lit/src/host/types.js';
import type {
  FrontendSnapshotPayload,
  LedgerRecord,
  ExecutionPlan,
  PendingToolPermission,
} from '../lit/src/protocol/types.js';

export const scenarios = [
  ['overview', '完整展示'],
  ['widgets', '交互表单'],
  ['tools', '工具状态'],
  ['approval', '多工具审批'],
  ['stream', '流式回复'],
  ['plan', '执行计划'],
  ['error', '异常与重试'],
  ['empty', '空会话'],
] as const;
type Session = {
  records: LedgerRecord[];
  plan: ExecutionPlan | null;
  permissions: PendingToolPermission[];
  modelUid: number;
  efforts: Record<number, ReasoningEffort | null>;
  permissionMode: string;
  paused: boolean;
};
const image = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="320"><rect width="640" height="320" fill="#e4efeb"/><path d="M80 240L220 100 350 230 440 140 560 240" fill="none" stroke="#167d71" stroke-width="8"/><text x="40" y="50" font-size="24" fill="#173d36">Orbit · Image preview</text></svg>')}`;
const widgetText =
  '### 交互表单\n\n请选择并填写，所有答案一次提交。\n\n[select:single | label="输出格式" | options="PDF,Word,Markdown"]\n[select:multi | label="处理范围" | options="正文,表格,图片"]\n[input:text | label="补充要求"]\n[input:path | label="输入文件" | accept=".xlsx,.csv"]\n[confirm | label="确认处理"]';
const overviewText = `## 前端验收\n\n这里可以检查 **Markdown**、*强调*、~~删除线~~、[链接](https://example.com)、图片、公式和图表。\n\n> Lit 与 Svelte 使用相同数据；操作不会执行真实业务。\n\n| 类型 | 当前状态 |\n| --- | --- |\n| 消息与布局 | 可验收 |\n| 模型及历史 | 本地模拟 |\n\n- 切换场景查看其他状态\n- 发送任意文字体验流式回复\n\n\`\`\`json\n{"workflow":"example","revision":1}\n\`\`\`\n\n行内公式 $E=mc^2$，块级公式：\n\n$$\\int_0^1 x^2\\,dx=\\frac{1}{3}$$\n\n\`\`\`mermaid\nflowchart LR\n  Host --> Transport --> State --> UI\n\`\`\`\n\n![验收图片](${image})`;

export class AcceptanceEnvironment implements ConversationTransport {
  readonly contract = TRANSPORT_CONTRACT;
  readonly id = 'acceptance-local';
  private handlers: ConversationTransportHandlers | null = null;
  private sessions = new Map<string, Session>();
  private current = 'acceptance-main';
  private revision = 0;
  private sequence = 0;
  private timer: ReturnType<typeof setInterval> | undefined;
  private state = 'waiting';
  private stream: FrontendSnapshotPayload['assistant_stream'] = null;
  private listeners = new Set<(event: ConversationPersistenceEvent) => void>();
  private images = new Map<string, string>();
  private archives: PersistedConversation[] = [];
  private snapshots = new Map<string, Session>();
  private definitions: ProviderDefinitionsResult = {
    current_model_uid: 1001,
    providers: [
      {
        uid: 1,
        name: '验收 Chat（模拟）',
        type: 'openai-gpt',
        base_url: 'https://example.invalid/v1',
        api_paradigm: 'openai_chat_completions',
        api_key_set: true,
      },
      {
        uid: 2,
        name: '验收 Responses（模拟）',
        type: 'openai-gpt',
        base_url: 'https://example.invalid/v1',
        api_paradigm: 'openai_responses',
        api_key_set: true,
      },
      {
        uid: 3,
        name: '验收 Claude（模拟）',
        type: 'claude',
        base_url: 'https://example.invalid',
        api_paradigm: 'anthropic_messages',
        api_key_set: true,
      },
    ],
    models: [
      {
        uid: 1001,
        provider_uid: 1,
        model_id: 'chat-demo',
        context_window: 128000,
        reasoning: { effort: 'medium' },
      },
      {
        uid: 1002,
        provider_uid: 2,
        model_id: 'responses-demo',
        context_window: 128000,
        reasoning: { effort: 'high', summary: 'auto' },
      },
      {
        uid: 1003,
        provider_uid: 3,
        model_id: 'claude-demo',
        context_window: 200000,
        reasoning: { effort: 'high', thinking: 'adaptive' },
      },
    ],
  };
  onAudit: (message: string) => void = () => {};
  failNextSend = false;
  failNextImage = false;
  constructor() {
    this.sessions.set(this.current, this.fixture('overview'));
    for (const [index, title] of [
      '已保存的验收会话',
      '后台会话示例',
    ].entries()) {
      const archiveId = `fixture-${index}`;
      this.archives.push({
        archiveId,
        title,
        runtimeConversationId: null,
        preview: '本地验收数据，可打开后继续发送消息。',
        updatedAt: new Date().toISOString(),
        status: 'sealed',
        instanceState: index ? 'background' : 'data_only',
      });
      this.snapshots.set(archiveId, this.fixture(index ? 'plan' : 'widgets'));
    }
  }
  private record(
    role: LedgerRecord['role'],
    content: string,
    metadata: LedgerRecord['metadata'] = {},
  ): LedgerRecord {
    return { record_id: `fixture-${++this.sequence}`, role, content, metadata };
  }
  private fixture(name: string): Session {
    const plan: ExecutionPlan = {
      plan_id: `plan-${++this.sequence}`,
      revision: 1,
      title: '验收会话功能',
      status: 'active',
      steps: [
        { id: 'a', text: '检查消息和富内容', status: 'completed' },
        { id: 'b', text: '检查审批与输入', status: 'in_progress' },
        { id: 'c', text: '检查模型、历史和外观', status: 'pending' },
      ],
    };
    const session: Session = {
      records: [],
      plan: null,
      permissions: [],
      modelUid: 1001,
      efforts: {},
      permissionMode: 'confirm',
      paused: false,
    };
    if (name === 'overview') {
      session.records = [
        this.record('user', '请参考这张图片，展示这次验收的内容。', {
          extra: {
            parts: [
              { type: 'text', text: '请参考这张图片 ' },
              { type: 'image', image_id: 'acceptance-example' },
              { type: 'text', text: ' 展示这次验收的内容。' },
            ],
          },
        }),
        this.record('assistant', overviewText),
      ];
      session.plan = plan;
      this.toolGallery(session, false);
      session.records.push(this.record('assistant', widgetText));
    } else if (name === 'widgets')
      session.records = [this.record('assistant', widgetText)];
    else if (name === 'tools') this.toolGallery(session, true);
    else if (name === 'plan') {
      session.records = [
        this.record(
          'assistant',
          '执行计划会在下方持续更新。可折叠后点击“推进计划”，检查展开状态是否保持。',
        ),
      ];
      session.plan = plan;
    } else if (name === 'error')
      session.records = [
        this.record(
          'assistant',
          '下面展示模型错误和中断消息。可开启“下次发送失败”检查输入内容是否保留。',
        ),
        this.record(
          'gateway_message',
          '模拟错误：模型服务暂时不可用，请重试。',
          { subtype: 'llm_error' },
        ),
        this.record('gateway_message', '模拟中断：本次执行已暂停。', {
          subtype: 'interrupted',
        }),
      ];
    else if (name === 'approval') {
      for (const [index, tool] of [
        'InspectFixture',
        'UpdateFixture',
      ].entries()) {
        const callId = `approval-${++this.sequence}`;
        session.records.push(
          this.record(
            'assistant',
            index ? '需要你的批准后才能更新示例资源。' : '先读取示例资源。',
            {
              extra: {
                tool_calls: [
                  { id: callId, function: { name: tool, arguments: '{}' } },
                ],
              },
            },
          ),
        );
        session.records.push(
          this.record('gateway_message', '等待批准', {
            subtype: 'tool_call_permission_requested',
            tool_name: tool,
            extra: { call_id: callId, status: 'waiting_permission' },
          }),
        );
        session.permissions.push({
          conversation_id: this.current,
          agent_id: 'demo-agent',
          tool_call_id: callId,
          tool_name: tool,
          display_name: index ? '更新示例资源' : '检查示例资源',
          effect: index ? 'controlled_change' : 'read_only',
          arguments: {
            path: 'fixtures/report.md',
            api_key: 'never-display',
            content: { internal: true },
          },
        });
      }
    }
    return session;
  }
  private toolGallery(session: Session, interactive: boolean) {
    const states = [
      ['finished', '读取示例文件', 'ReadFixture', '已读取 12 行。'],
      [
        'failed',
        '更新示例文件',
        'UpdateFixture',
        '模拟失败：示例文件为只读，请检查权限后重试。',
      ],
      ['placeholder', '导出报告', 'ExportFixture', ''],
      ['running', '查询示例数据', 'QueryFixture', '正在查询示例数据。'],
      [
        'waiting_permission',
        '写入示例报告',
        'SaveFixture',
        '等待用户批准后写入 fixtures/report.md。',
      ],
      [
        'uncertain',
        '恢复中断的任务',
        'RecoverFixture',
        '执行中断，结果需要确认。',
      ],
    ] as const;
    session.records.push(
      this.record(
        'assistant',
        '### 工具状态\n\n展开工具可查看面向用户的状态说明。完整展示中的状态为模拟快照；切换“工具状态”可操作审批。',
      ),
    );
    for (const [status, title, tool, detail] of states) {
      const id = `tool-${++this.sequence}`;
      session.records.push(
        this.record('assistant', '', {
          extra: {
            tool_calls: [{ id, function: { name: tool, arguments: '{}' } }],
          },
        }),
      );
      if (status !== 'placeholder')
        session.records.push(
          this.record('tool', detail, {
            title,
            tool_name: tool,
            tool_command: `${tool} --path fixtures/report.md`,
            subtype:
              status === 'finished'
                ? 'tool_call_finished'
                : status === 'failed'
                  ? 'tool_call_failed'
                  : status === 'waiting_permission'
                    ? 'tool_call_permission_requested'
                    : status === 'uncertain'
                      ? 'tool_call_interrupted_unknown'
                      : 'tool_call_started',
            extra: { call_id: id, status, user_summary: detail || undefined },
          }),
        );
      if (interactive && status === 'waiting_permission')
        session.permissions.push({
          conversation_id: this.current,
          agent_id: 'demo-agent',
          tool_call_id: id,
          tool_name: tool,
          display_name: title,
          effect: 'controlled_change',
          arguments: { path: 'fixtures/report.md' },
        });
    }
  }
  private get session() {
    return this.sessions.get(this.current)!;
  }
  async connect(
    context: { conversationId?: string | null },
    handlers: ConversationTransportHandlers,
  ) {
    this.stop();
    this.handlers = handlers;
    this.current = context.conversationId ?? `acceptance-${++this.sequence}`;
    if (!this.sessions.has(this.current))
      this.sessions.set(this.current, this.fixture('overview'));
    this.state = this.session.permissions.length ? 'executing' : 'waiting';
    const bound = this.current;
    handlers.connection('connected');
    handlers.event({ type: 'conversation-created', conversationId: bound });
    this.publish();
    this.onAudit(`连接会话 ${bound}`);
    return {
      conversationId: bound,
      disconnect: () => {
        if (this.handlers === handlers) {
          this.stop();
          this.handlers = null;
        }
      },
    };
  }
  private publish() {
    this.handlers?.event({
      type: 'state-snapshot',
      conversationId: this.current,
      payload: {
        revision: ++this.revision,
        conversation_state: this
          .state as FrontendSnapshotPayload['conversation_state'],
        active_agent_id: 'demo-agent',
        ledger_records: structuredClone(this.session.records),
        pending_permissions: structuredClone(this.session.permissions),
        plan: structuredClone(this.session.plan),
        assistant_stream: this.stream,
        model_uid: this.session.modelUid,
        model: this.modelName(this.session.modelUid),
      },
    });
  }
  private modelName(uid: number) {
    return (
      this.definitions.models?.find((m) => m.uid === uid)?.model_id ??
      `Model ${uid}`
    );
  }
  private stop() {
    clearInterval(this.timer);
    this.timer = undefined;
    this.stream = null;
    this.state = 'waiting';
  }
  scenario(name: string) {
    this.stop();
    this.session.records = [];
    this.sessions.set(this.current, this.fixture(name));
    this.state = this.session.permissions.length ? 'executing' : 'waiting';
    this.publish();
    this.onAudit(`场景：${scenarios.find((s) => s[0] === name)?.[1] ?? name}`);
    if (name === 'stream')
      this.startStream(
        '这是一段流式回复。你可以观察逐段显示、暂停按钮、滚动跟随以及回复完成后的输入状态。',
      );
  }
  private startStream(text: string) {
    this.stop();
    this.state = 'thinking';
    let length = 0;
    const turn = ++this.sequence;
    this.publish();
    this.timer = setInterval(() => {
      length = Math.min(text.length, length + 2);
      this.stream = {
        agent_id: 'demo-agent',
        turn_id: turn,
        attempt: 1,
        sequence: length,
        content: text.slice(0, length),
      };
      if (length === text.length) {
        this.session.records.push(this.record('assistant', text));
        this.stop();
      }
      this.publish();
    }, 100);
  }
  async send(request: SendMessageRequest) {
    if (this.failNextSend) {
      this.failNextSend = false;
      this.onAudit('模拟发送失败');
      return {
        accepted: false,
        rejectReason: '模拟发送失败，请保留内容后重试。',
      };
    }
    if (this.state !== 'waiting')
      return { accepted: false, rejectReason: '请先暂停或完成当前操作。' };
    this.session.records.push(
      this.record(
        'user',
        request.content,
        request.parts ? { extra: { parts: request.parts } } : {},
      ),
    );
    this.session.paused = false;
    this.onAudit(
      `发送：${request.content}${request.parts ? `（${request.parts.length} 个片段）` : ''}`,
    );
    this.startStream(
      `已收到：${request.content}\n\n这是本地模拟回复，便于验收流式显示和输入状态。你可以切换“多工具审批”“交互表单”等场景继续检查。`,
    );
    return { accepted: true };
  }
  imageInput = {
    importImage: async ({ file }: { file: File }) => {
      if (this.failNextImage) {
        this.failNextImage = false;
        throw new Error('模拟图片上传失败，请重试。');
      }
      const imageId = `image-${++this.sequence}`;
      this.images.set(imageId, URL.createObjectURL(file));
      this.onAudit(`导入图片：${file.name}`);
      return { imageId };
    },
  };
  async pause() {
    const partial = this.stream?.content;
    this.stop();
    if (partial) this.session.records.push(this.record('assistant', partial));
    for (const p of this.session.permissions)
      this.finishTool(p.tool_call_id, p.tool_name, false);
    this.session.permissions = [];
    this.session.paused = true;
    this.publish();
    this.onAudit('暂停');
    return { accepted: true };
  }
  async resume() {
    if (this.state !== 'waiting' || !this.session.paused)
      return { accepted: false, rejectReason: '当前会话不可继续。' };
    this.session.paused = false;
    this.startStream(
      '已继续上次暂停的任务。这是本地模拟回复，用于验收继续按钮和流式状态。',
    );
    this.onAudit('继续执行');
    return { accepted: true };
  }
  composerController: ConversationComposerController = {
    getSettings: async ({ conversationId, modelUid }) => {
      const session = this.sessions.get(conversationId);
      if (!session) throw new Error('会话不存在。');
      return {
        effort: session.efforts[modelUid ?? session.modelUid] ?? null,
        supportedEfforts:
          modelUid === 1003
            ? ['low', 'medium', 'high']
            : ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'],
        permissionMode: session.permissionMode,
        permissionModes: [
          {
            id: 'confirm',
            label: '逐项批准',
            description: '工具执行前由你批准。',
          },
          {
            id: 'assist',
            label: '帮我批准',
            description: '自动批准只读操作；修改操作仍需你批准。',
          },
        ],
        canResume: session.paused,
      };
    },
    setEffort: async ({ conversationId, modelUid, effort }) => {
      const session = this.sessions.get(conversationId);
      if (
        !session ||
        (conversationId === this.current && this.state !== 'waiting')
      )
        return { accepted: false, rejectReason: '运行中不能修改推理强度。' };
      session.efforts[modelUid ?? session.modelUid] = effort;
      this.onAudit(`推理强度：${effort ?? '默认'}（仅当前会话）`);
      return { accepted: true };
    },
    setPermissionMode: async ({ conversationId, mode }) => {
      const session = this.sessions.get(conversationId);
      if (
        !session ||
        !['confirm', 'assist'].includes(mode) ||
        (conversationId === this.current && this.state !== 'waiting')
      )
        return { accepted: false, rejectReason: '当前不能修改权限模式。' };
      session.permissionMode = mode;
      this.onAudit(`权限模式：${mode}`);
      return { accepted: true };
    },
  };
  private finishTool(id: string, tool: string, allowed: boolean) {
    this.session.records.push(
      this.record('tool', allowed ? '示例操作完成。' : '示例操作已取消。', {
        subtype: allowed ? 'tool_call_finished' : 'tool_call_failed',
        tool_name: tool,
        extra: { call_id: id },
      }),
    );
  }
  async resolveToolPermission(request: {
    toolCallId: string;
    decision: 'allow' | 'deny';
  }) {
    const permission = this.session.permissions.find(
      (p) => p.tool_call_id === request.toolCallId,
    );
    if (!permission)
      return {
        accepted: false,
        metadata: { resolved: false },
        rejectReason: 'Permission request is no longer pending.',
      };
    this.finishTool(
      permission.tool_call_id,
      permission.tool_name,
      request.decision === 'allow',
    );
    this.session.permissions = this.session.permissions.filter(
      (p) => p !== permission,
    );
    this.state = this.session.permissions.length ? 'executing' : 'waiting';
    this.publish();
    this.onAudit(
      `${request.decision === 'allow' ? '允许' : '拒绝'}：${permission.tool_name}`,
    );
    return { accepted: true };
  }
  async requestSnapshot() {
    this.publish();
  }
  async close() {
    this.stop();
    this.handlers?.event({
      type: 'conversation-closed',
      conversationId: this.current,
    });
    this.onAudit('关闭会话');
    return { accepted: true };
  }
  expireApprovals() {
    for (const p of [...this.session.permissions]) {
      this.finishTool(p.tool_call_id, p.tool_name, false);
      this.handlers?.event({
        type: 'transport-extension',
        extension: {
          contract: 'agent-conversation-extension/v1',
          namespace: 'runtime',
          version: 1,
          kind: 'tool:permission-resolved',
          payload: { tool_call_id: p.tool_call_id },
        },
      });
    }
    this.session.permissions = [];
    this.state = 'waiting';
    this.publish();
    this.onAudit('审批已过期');
  }
  advancePlan() {
    const plan = this.session.plan;
    if (!plan) return;
    const next = plan.steps?.find((s) => s.status !== 'completed');
    if (next) next.status = 'completed';
    const following = plan.steps?.find((s) => s.status === 'pending');
    if (following) following.status = 'in_progress';
    plan.revision = (plan.revision ?? 0) + 1;
    if (plan.steps?.every((s) => s.status === 'completed'))
      plan.status = 'finished';
    this.publish();
  }
  compact() {
    this.stop();
    this.state = 'compacting';
    this.publish();
    this.timer = setInterval(() => {
      this.stop();
      this.publish();
    }, 1600);
  }
  providerController: ConversationProviderController = {
    getProviderDefinitions: async () => structuredClone(this.definitions),
    getBuiltinProviderCatalog: async () => ({
      providers: [
        {
          id: 'openai-gpt',
          name: 'OpenAI Compatible',
          defaultBaseUrl: 'https://example.invalid/v1',
          apiFormat: 'openai',
        },
        {
          id: 'claude',
          name: 'Anthropic Compatible',
          defaultBaseUrl: 'https://example.invalid',
          apiFormat: 'anthropic',
        },
      ],
      models: [
        { id: 'chat-demo', name: '验收 Chat', contextWindow: 128000 },
        { id: 'claude-demo', name: '验收 Claude', contextWindow: 200000 },
      ],
    }),
    setConversationModel: async ({ conversationId, modelUid }) => {
      const session = this.sessions.get(conversationId);
      if (!session) return { accepted: false, rejectReason: '会话不存在。' };
      session.modelUid = modelUid;
      if (conversationId === this.current) this.publish();
      this.onAudit(`切换模型：${modelUid}`);
      return { accepted: true };
    },
    configureProviders: async ({ input }) => {
      const payload = JSON.parse(input);
      if (!Array.isArray(payload.providers) || !payload.providers.length)
        throw new Error('至少提供一个厂商。');
      const providers: NonNullable<ProviderDefinitionsResult['providers']> = [];
      const models: NonNullable<ProviderDefinitionsResult['models']> = [];
      for (const p of payload.providers) {
        if (!p.name || !p.base_url || !p.enabled_models?.length)
          throw new Error('厂商名称、接口地址和模型不能为空。');
        providers.push({
          uid: p.id,
          name: p.name,
          type: p.type,
          base_url: p.base_url,
          api_paradigm: p.api_paradigm,
          api_key_set:
            Boolean(p.api_key) ||
            Boolean(
              this.definitions.providers?.find((old) => old.uid === p.id)
                ?.api_key_set,
            ),
        });
        for (const m of p.enabled_models)
          models.push({
            uid: m.uid,
            provider_uid: p.id,
            model_id: m.model_id,
            context_window: m.max_context_tokens,
            reasoning: m.reasoning,
          });
      }
      this.definitions = {
        providers,
        models,
        current_model_uid: payload.current_model_uid,
      };
      this.onAudit('厂商配置已保存（仅当前验收页面，不发送密钥）');
      return { accepted: true };
    },
  };
  persistenceController: ConversationPersistenceController = {
    contract: PERSISTENCE_CONTRACT,
    list: async () => ({ items: structuredClone(this.archives) }),
    create: async () => {
      const runtimeConversationId = `acceptance-${++this.sequence}`;
      this.sessions.set(runtimeConversationId, this.fixture('empty'));
      return { runtimeConversationId };
    },
    save: async ({ archiveId, runtimeConversationId }) => {
      const session = this.sessions.get(runtimeConversationId);
      if (!session) throw new Error('会话不存在。');
      const id = archiveId ?? `saved-${++this.sequence}`;
      const archive: PersistedConversation = {
        archiveId: id,
        runtimeConversationId,
        title: `验收会话 ${id}`,
        preview: session.records.at(-1)?.content?.slice(0, 70) ?? '空会话',
        status: 'sealed',
        instanceState: 'current',
        updatedAt: new Date().toISOString(),
      };
      this.archives = [
        archive,
        ...this.archives.filter((a) => a.archiveId !== id),
      ];
      this.snapshots.set(id, structuredClone(session));
      for (const listener of this.listeners)
        listener({ type: 'archive-updated', archive });
      this.onAudit('会话已保存到当前页面内存');
      return archive;
    },
    restore: async ({ archiveId }) => {
      const session = this.snapshots.get(archiveId);
      if (!session) throw new Error('归档不存在。');
      const runtimeConversationId = `restored-${++this.sequence}`;
      this.sessions.set(runtimeConversationId, structuredClone(session));
      this.onAudit(`恢复归档：${archiveId}`);
      return { archiveId, runtimeConversationId };
    },
    subscribe: (listener) => {
      this.listeners.add(listener);
      return () => this.listeners.delete(listener);
    },
  };
  capabilities: ConversationHostCapabilities = {
    resolveImage: async ({ imageId }) => {
      const source =
        imageId === 'acceptance-example' ? image : this.images.get(imageId);
      return source ? { source, alt: '预览图片' } : null;
    },
    openLink: async ({ url }) => {
      window.open(url, '_blank', 'noopener,noreferrer');
      this.onAudit(`打开链接：${url}`);
    },
    openImage: async ({ source }) => {
      const dialog = document.createElement('dialog');
      const img = document.createElement('img');
      img.src = this.images.get(source) ?? source;
      img.alt = '图片预览';
      img.style.cssText = 'max-width:80vw;max-height:75vh;display:block';
      const close = document.createElement('button');
      close.textContent = '关闭';
      close.onclick = () => dialog.close();
      dialog.append(img, close);
      dialog.addEventListener('close', () => dialog.remove(), { once: true });
      document.body.append(dialog);
      dialog.showModal();
      this.onAudit('打开图片预览');
    },
    pickPath: async () => {
      this.onAudit('选择路径（模拟）');
      return { paths: ['fixtures/example.xlsx'] };
    },
    copyText: async (text) => {
      await navigator.clipboard.writeText(text);
      this.onAudit('已复制');
    },
    download: async ({ source, suggestedName }) => {
      const a = document.createElement('a');
      a.href = source;
      a.download = suggestedName ?? 'example';
      a.click();
    },
  };
  dispose() {
    this.stop();
    for (const url of this.images.values()) URL.revokeObjectURL(url);
    this.images.clear();
    this.listeners.clear();
  }
}

export type AcceptanceApi = AgentRuntimeConversationPublicApi & {
  hideToolCalls: boolean;
  progressiveReveal: boolean;
  disclaimer: string;
};
export function mountAcceptanceControls(
  root: HTMLElement,
  api: AcceptanceApi,
  environment: AcceptanceEnvironment,
  label: string,
) {
  root.innerHTML = `<div class="acceptance-heading"><strong>${label} 前端验收</strong><span>本地模拟 · 不连接真实模型或业务工具</span><a href="${label === 'Lit' ? 'http://127.0.0.1:1421/' : 'http://127.0.0.1:1420/'}" target="_blank">打开 ${label === 'Lit' ? 'Svelte' : 'Lit'}</a></div><div class="acceptance-fields"><label>场景<select name="scenario">${scenarios.map(([id, name]) => `<option value="${id}">${name}</option>`).join('')}</select></label><label>主题<select name="theme">${['white', 'studio', 'paper', 'soft', 'midnight', 'neutral', 'blue', 'green', 'amber'].map((t) => `<option>${t}</option>`).join('')}</select></label><label>配色<select name="colorScheme"><option value="light">浅色</option><option value="dark">深色</option><option value="system">跟随系统</option></select></label><label>密度<select name="density"><option value="comfortable">舒适</option><option value="compact">紧凑</option></select></label><label>语言<select name="locale"><option value="zh-CN">中文</option><option value="en-US">English</option></select></label></div><div class="acceptance-actions"><button data-action="preset">插入预设消息</button><button data-action="save">保存会话</button><button data-action="plan">推进计划</button><button data-action="expire">审批过期</button><button data-action="compact">总结状态</button><button data-action="close">关闭会话</button><button data-action="connect">重新连接</button><label><input type="checkbox" name="hideToolCalls">隐藏工具</label><label><input type="checkbox" name="progressiveReveal" checked>渐进显示</label><label><input type="checkbox" name="failNextSend">下次发送失败</label><label><input type="checkbox" name="failNextImage">下次图片失败</label></div><details class="acceptance-audit"><summary>操作记录</summary><ol></ol></details>`;
  root.querySelector<HTMLSelectElement>('[name="theme"]')!.value = api.theme;
  const logs = root.querySelector('ol')!;
  const audit = (message: string) => {
    const li = document.createElement('li');
    li.textContent = `${new Date().toLocaleTimeString()} ${message}`;
    logs.prepend(li);
    while (logs.children.length > 40) logs.lastElementChild?.remove();
  };
  environment.onAudit = audit;
  const change = (event: Event) => {
    const input = event.target as HTMLInputElement | HTMLSelectElement;
    const name = input.name;
    if (name === 'scenario') {
      api.clearPresentationItems();
      environment.scenario(input.value);
    } else if (name === 'failNextSend' || name === 'failNextImage')
      environment[name] = (input as HTMLInputElement).checked;
    else if (name === 'hideToolCalls' || name === 'progressiveReveal')
      api[name] = (input as HTMLInputElement).checked;
    else if (name === 'theme') api.theme = input.value;
    else if (name === 'density')
      api.density = input.value as AcceptanceApi['density'];
    else if (name === 'colorScheme')
      api.colorScheme = input.value as AcceptanceApi['colorScheme'];
    else if (name === 'locale') api.locale = input.value;
  };
  const click = async (event: Event) => {
    const action = (event.target as Element).closest<HTMLButtonElement>(
      'button[data-action]',
    )?.dataset.action;
    if (!action) return;
    try {
      if (action === 'preset')
        api.insertPresetMarkdown({
          markdown:
            '### 预设回复\n\n这段内容由宿主插入，仅用于前端展示，可观察渐进显示。',
        });
      else if (action === 'save') await api.saveConversation();
      else if (action === 'plan') environment.advancePlan();
      else if (action === 'expire') environment.expireApprovals();
      else if (action === 'compact') environment.compact();
      else if (action === 'close') await api.closeConversation();
      else if (action === 'connect') await api.connect();
    } catch (error) {
      audit(`操作失败：${String(error)}`);
    }
  };
  root.addEventListener('change', change);
  root.addEventListener('click', click);
  audit('验收场景就绪。可直接发送消息，或从场景列表切换。');
  return () => {
    root.removeEventListener('change', change);
    root.removeEventListener('click', click);
    environment.onAudit = () => {};
  };
}
