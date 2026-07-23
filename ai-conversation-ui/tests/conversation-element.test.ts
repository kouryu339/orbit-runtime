// @vitest-environment happy-dom

import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  AgentRuntimeConversationElement,
  PERSISTENCE_CONTRACT,
  PRESENTATION_CONTRACT,
  TRANSPORT_CONTRACT,
  conversationReducer,
  type ConversationConnection,
  type ConversationPersistenceController,
  type ConversationProviderController,
  type ConversationTransport,
  type ConversationTransportHandlers,
  type SendMessageRequest,
} from '../src/index.js';

class TestTransport implements ConversationTransport {
  readonly contract = TRANSPORT_CONTRACT;
  readonly id = 'test';
  disconnect = vi.fn();
  resolveToolPermission = vi.fn(async (): Promise<{
    accepted: boolean;
    rejectReason?: string;
  }> => ({ accepted: true }));

  async connect(
    _context: unknown,
    handlers: ConversationTransportHandlers,
  ): Promise<ConversationConnection> {
    handlers.connection('connected');
    handlers.event({
      type: 'conversation-created',
      conversationId: 'conversation-1',
    });
    handlers.event({
      type: 'state-snapshot',
      conversationId: 'conversation-1',
      payload: {
        revision: 1,
        conversation_state: 'waiting',
        ledger_records: [{
          record_id: 'assistant-1',
          role: 'assistant',
          content: 'Ready',
        }],
      },
    });
    return {
      conversationId: 'conversation-1',
      disconnect: this.disconnect,
    };
  }

  async send(request: SendMessageRequest) {
    return { accepted: Boolean(request.content) };
  }
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe('AgentRuntimeConversationElement', () => {
  function persistenceController(): ConversationPersistenceController {
    return {
      contract: PERSISTENCE_CONTRACT,
      list: vi.fn(async () => ({
        items: [{
          archiveId: 'archive-1',
          runtimeConversationId: null,
          title: 'Saved conversation',
          status: 'sealed' as const,
        }],
      })),
      save: vi.fn(async (request) => ({
        archiveId: request.archiveId ?? 'archive-1',
        runtimeConversationId: request.runtimeConversationId,
        title: 'Saved conversation',
        status: 'running' as const,
      })),
      create: vi.fn(async () => ({
        runtimeConversationId: 'runtime-new-1',
      })),
      restore: vi.fn(async () => ({
        archiveId: 'archive-1',
        runtimeConversationId: 'restored-runtime-2',
      })),
    };
  }

  it('connects, renders, sends, and releases the transport', async () => {
    const element = new AgentRuntimeConversationElement();
    const transport = new TestTransport();
    element.transport = transport;
    element.progressiveReveal = false;
    document.body.append(element);

    await element.connect();
    await element.updateComplete;

    expect(element.state.conversationId).toBe('conversation-1');
    expect(element.state.runtimeState).toBe('waiting');
    const content = element.shadowRoot?.querySelector(
      'agent-conversation-rich-content',
    ) as HTMLElement & { updateComplete: Promise<boolean> };
    await content.updateComplete;
    expect(content.shadowRoot?.textContent).toContain('Ready');
    await expect(element.send('hello')).resolves.toMatchObject({ accepted: true });

    element.remove();
    expect(transport.disconnect).toHaveBeenCalledOnce();
  });

  it('renders a permission request and returns the decision through the transport', async () => {
    const element = new AgentRuntimeConversationElement();
    const transport = new TestTransport();
    element.transport = transport;
    document.body.append(element);
    await element.connect();

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        pending_permissions: [{
          conversation_id: 'conversation-1',
          tool_call_id: 'call-1',
          agent_id: 'boss',
          tool_name: 'WriteFile',
          display_name: 'Write file',
          effect: 'controlled_change',
          arguments: { path: 'report.md' },
          turn_id: 2,
          created_at: '2026-06-22T00:00:00Z',
        }],
      },
    });
    await element.updateComplete;

    const shelf = element.shadowRoot?.querySelector('[part="permission-shelf"]');
    expect(shelf?.textContent).toContain('Write file');
    const deny = element.shadowRoot?.querySelector<HTMLButtonElement>(
      '.permission-actions .deny',
    );
    deny?.click();
    await vi.waitFor(() => {
      expect(transport.resolveToolPermission).toHaveBeenCalledWith({
        conversationId: 'conversation-1',
        toolCallId: 'call-1',
        decision: 'deny',
      });
    });
  });

  it('keeps a rejected permission visible and shows the transport reason', async () => {
    const element = new AgentRuntimeConversationElement();
    const transport = new TestTransport();
    transport.resolveToolPermission.mockResolvedValueOnce({
      accepted: false,
      rejectReason: 'Runtime could not find the target approval.',
    });
    element.transport = transport;
    document.body.append(element);
    await element.connect();

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        pending_permissions: [{
          conversation_id: 'conversation-1',
          tool_call_id: 'call-rejected',
          agent_id: 'boss',
          tool_name: 'BrowserClosePage',
          display_name: 'Close page',
          effect: 'controlled_change',
          arguments: { page_id: 'page-with-a-very-long-identifier' },
          turn_id: 2,
          created_at: '2026-06-22T00:00:00Z',
        }],
      },
    });
    await element.updateComplete;

    element.shadowRoot
      ?.querySelector<HTMLButtonElement>('.permission-actions button:not(.deny)')
      ?.click();
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector('[role="alert"]')?.textContent)
        .toContain('Runtime could not find the target approval.');
    });
    expect(element.state.pendingPermissions).toHaveLength(1);
    expect(element.shadowRoot?.querySelector('[part="permission-shelf"]')).not.toBeNull();
  });

  it('keeps approval actions in the shelf when the tool call is rendered', async () => {
    const element = new AgentRuntimeConversationElement();
    const transport = new TestTransport();
    element.transport = transport;
    document.body.append(element);
    await element.connect();

    const permission = {
      conversation_id: 'conversation-1',
      tool_call_id: 'call-1',
      agent_id: 'boss',
      tool_name: 'WriteFile',
      display_name: 'Write file',
      effect: 'controlled_change' as const,
      arguments: { path: 'report.md' },
      turn_id: 2,
      created_at: '2026-06-22T00:00:00Z',
    };
    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'executing',
        pending_permissions: [permission],
        ledger_records: [
          {
            record_id: 'assistant-2',
            role: 'assistant',
            content: '[tool:status | call_id="call-1"]',
          },
          {
            record_id: 'tool-1',
            role: 'gateway_message',
            content: 'Waiting for permission: WriteFile',
            metadata: {
              subtype: 'tool_call_permission_requested',
              title: 'Write file',
              extra: { call_id: 'call-1', status: 'waiting_permission' },
            },
          },
        ],
      },
    });
    await element.updateComplete;

    const shelf = element.shadowRoot?.querySelector('[part="permission-shelf"]');
    expect(shelf?.textContent).toContain('Write file');
    const richContent = element.shadowRoot?.querySelector(
      'agent-conversation-rich-content',
    ) as HTMLElement & { updateComplete: Promise<boolean>; shadowRoot: ShadowRoot };
    await richContent.updateComplete;
    expect(richContent.shadowRoot.textContent).toContain('Write file');

    expect(richContent.shadowRoot.querySelector('.tool-permission-actions')).toBeNull();
    const allow = shelf?.querySelector<HTMLButtonElement>(
      '.permission-actions button:not(.deny)',
    );
    allow?.click();
    await vi.waitFor(() => {
      expect(transport.resolveToolPermission).toHaveBeenCalledWith({
        conversationId: 'conversation-1',
        toolCallId: 'call-1',
        decision: 'allow',
      });
    });
    await element.updateComplete;
    expect(element.state.pendingPermissions).toEqual([]);
    expect(element.shadowRoot?.querySelector('[part="permission-shelf"]')).toBeNull();
  });

  it('renders multiple permission requests in the composer shelf', async () => {
    const element = new AgentRuntimeConversationElement();
    const transport = new TestTransport();
    element.transport = transport;
    document.body.append(element);
    await element.connect();

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'executing',
        pending_permissions: [
          {
            conversation_id: 'conversation-1',
            tool_call_id: 'call-1',
            agent_id: 'boss',
            tool_name: 'WriteFile',
            display_name: 'Write file',
            effect: 'controlled_change',
            arguments: { path: 'report.md' },
            turn_id: 2,
            created_at: '2026-06-22T00:00:00Z',
          },
          {
            conversation_id: 'conversation-1',
            tool_call_id: 'call-2',
            agent_id: 'boss',
            tool_name: 'DeleteFile',
            display_name: 'Delete file',
            effect: 'destructive',
            arguments: { path: 'old.md' },
            turn_id: 2,
            created_at: '2026-06-22T00:00:01Z',
          },
        ],
      },
    });
    await element.updateComplete;

    const shelf = element.shadowRoot?.querySelector('[part="permission-shelf"]');
    expect(shelf?.textContent).toContain('Write file');
    expect(shelf?.textContent).toContain('Delete file');
    const cards = element.shadowRoot?.querySelectorAll('.permission-tool');
    expect(cards).toHaveLength(2);

    const secondAllow = cards?.[1]?.querySelector<HTMLButtonElement>(
      '.permission-actions button:not(.deny)',
    );
    secondAllow?.click();
    await vi.waitFor(() => {
      expect(transport.resolveToolPermission).toHaveBeenCalledWith({
        conversationId: 'conversation-1',
        toolCallId: 'call-2',
        decision: 'allow',
      });
    });
  });

  it('lists every pending permission in the shelf regardless of tool rendering', async () => {
    const element = new AgentRuntimeConversationElement();
    const transport = new TestTransport();
    element.transport = transport;
    document.body.append(element);
    await element.connect();

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'executing',
        pending_permissions: [
          {
            conversation_id: 'conversation-1',
            tool_call_id: 'call-visible',
            agent_id: 'boss',
            tool_name: 'ReadFile',
            display_name: 'Read file',
            effect: 'read_only',
            arguments: { path: 'report.md' },
            turn_id: 2,
            created_at: '2026-06-22T00:00:00Z',
          },
          {
            conversation_id: 'conversation-1',
            tool_call_id: 'call-hidden',
            agent_id: 'boss',
            tool_name: 'WriteFile',
            display_name: 'Write file',
            effect: 'controlled_change',
            arguments: { path: 'report.md' },
            turn_id: 2,
            created_at: '2026-06-22T00:00:01Z',
          },
        ],
        ledger_records: [
          {
            record_id: 'assistant-2',
            role: 'assistant',
            content: '[tool:status | call_id="call-visible"]',
          },
          {
            record_id: 'tool-1',
            role: 'gateway_message',
            content: 'Waiting for permission: ReadFile',
            metadata: {
              subtype: 'tool_call_permission_requested',
              title: 'Read file',
              extra: { call_id: 'call-visible', status: 'waiting_permission' },
            },
          },
          {
            record_id: 'tool-2',
            role: 'gateway_message',
            content: 'Waiting for permission: WriteFile',
            metadata: {
              subtype: 'tool_call_permission_requested',
              title: 'Write file',
              extra: { call_id: 'call-hidden', status: 'waiting_permission' },
            },
          },
        ],
      },
    });
    await element.updateComplete;

    const shelf = element.shadowRoot?.querySelector('[part="permission-shelf"]');
    expect(shelf?.textContent).toContain('Write file');
    expect(shelf?.textContent).toContain('Read file');
    const richContent = element.shadowRoot?.querySelector(
      'agent-conversation-rich-content',
    ) as HTMLElement & { updateComplete: Promise<boolean>; shadowRoot: ShadowRoot };
    await richContent.updateComplete;
    expect(richContent.shadowRoot.textContent).toContain('Read file');
  });

  it('keeps the approval shelf visible when tool bubbles are hidden', async () => {
    const element = new AgentRuntimeConversationElement();
    const transport = new TestTransport();
    element.transport = transport;
    element.hideToolCalls = true;
    document.body.append(element);
    await element.connect();

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'executing',
        pending_permissions: [{
          conversation_id: 'conversation-1',
          tool_call_id: 'call-1',
          agent_id: 'boss',
          tool_name: 'WriteFile',
          display_name: 'Write file',
          effect: 'controlled_change',
          arguments: { path: 'report.md' },
          turn_id: 2,
          created_at: '2026-06-22T00:00:00Z',
        }],
        ledger_records: [
          {
            record_id: 'assistant-2',
            role: 'assistant',
            content: '[tool:status | call_id="call-1"]',
          },
          {
            record_id: 'tool-1',
            role: 'gateway_message',
            content: 'Waiting for permission: WriteFile',
            metadata: {
              subtype: 'tool_call_permission_requested',
              title: 'Write file',
              extra: { call_id: 'call-1', status: 'waiting_permission' },
            },
          },
        ],
      },
    });
    await element.updateComplete;

    const shelf = element.shadowRoot?.querySelector('[part="permission-shelf"]');
    expect(shelf?.textContent).toContain('Write file');
    const richContent = element.shadowRoot?.querySelector(
      'agent-conversation-rich-content',
    ) as HTMLElement & { updateComplete: Promise<boolean>; shadowRoot: ShadowRoot };
    await richContent.updateComplete;
    expect(richContent.shadowRoot.querySelector('[part="tool-call"]')).toBeNull();
  });

  it('switches theme without reconnecting', async () => {
    const element = new AgentRuntimeConversationElement();
    const transport = new TestTransport();
    element.transport = transport;
    document.body.append(element);
    await element.connect();
    element.theme = 'green';
    element.colorScheme = 'dark';
    await element.updateComplete;

    expect(element.getAttribute('theme')).toBe('green');
    expect(element.getAttribute('color-scheme')).toBe('dark');
    expect(transport.disconnect).not.toHaveBeenCalled();
  });

  it('keeps the composer disabled until the runtime is stopped', async () => {
    const element = new AgentRuntimeConversationElement();
    element.transport = new TestTransport();
    document.body.append(element);
    await element.connect();

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: { revision: 2, conversation_state: 'thinking' },
    });
    await element.updateComplete;

    const textarea = element.shadowRoot?.querySelector('textarea');
    expect(textarea?.disabled).toBe(true);
    await expect(element.send('too early')).resolves.toMatchObject({
      accepted: false,
    });
  });

  it('enables the composer after a revision-zero waiting snapshot', async () => {
    const element = new AgentRuntimeConversationElement();
    element.state = conversationReducer(element.state, {
      type: 'conversation-created',
      conversationId: 'conversation-1',
    });
    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: { revision: 0, conversation_state: 'waiting' },
    });
    document.body.append(element);
    await element.updateComplete;

    expect(element.shadowRoot?.querySelector('textarea')?.disabled).toBe(false);
  });

  it('shows loading while running before assistant output, even with a pending message', async () => {
    const element = new AgentRuntimeConversationElement();
    element.transport = new TestTransport();
    element.progressiveReveal = false;
    document.body.append(element);
    await element.connect();

    element.state = conversationReducer(element.state, {
      type: 'local-message-added',
      message: {
        id: 'local-1',
        content: 'hello',
        createdAt: new Date().toISOString(),
        state: 'sending',
      },
    });
    await element.updateComplete;
    expect(element.shadowRoot?.querySelector('.waiting')?.textContent).toContain('Loading');

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: {
        revision: 2,
        conversation_state: 'thinking',
        ledger_delta: {
          kind: 'append',
          record: { record_id: 'user-2', role: 'user', content: 'hello' },
        },
      },
    });
    await element.updateComplete;
    expect(element.shadowRoot?.querySelector('.waiting')?.textContent).toContain('Loading');

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: {
        revision: 3,
        conversation_state: 'thinking',
        ledger_delta: {
          kind: 'append',
          record: {
            record_id: 'assistant-2',
            role: 'assistant',
            content: 'Streaming reply',
          },
        },
      },
    });
    await element.updateComplete;
    expect(element.shadowRoot?.querySelector('.waiting')).toBeNull();
  });

  it('shows summarizing solely from the compacting state', async () => {
    const element = new AgentRuntimeConversationElement();
    element.state = {
      ...element.state,
      initialized: true,
      snapshotReceived: true,
      runtimeState: 'compacting',
      pendingUserMessages: [{
        id: 'local-1',
        content: 'hello',
        createdAt: new Date().toISOString(),
        state: 'accepted',
      }],
      turnObservedAssistant: true,
    };
    document.body.append(element);
    await element.updateComplete;

    expect(element.shadowRoot?.querySelector('.waiting')?.textContent)
      .toContain('总结中');
  });

  it('resets the composer height after sending', async () => {
    const element = new AgentRuntimeConversationElement();
    element.transport = new TestTransport();
    document.body.append(element);
    await element.connect();
    await element.updateComplete;

    const textarea = element.shadowRoot?.querySelector('textarea');
    expect(textarea).toBeTruthy();
    textarea!.style.height = '140px';

    await element.send('hello');
    await new Promise((resolve) => requestAnimationFrame(resolve));

    expect(textarea!.style.height).toBe('');
  });

  it('reveals queued local preset replies in arrival order', async () => {
    vi.useFakeTimers();
    try {
      const element = new AgentRuntimeConversationElement();
      element.transport = new TestTransport();
      document.body.append(element);
      await element.connect();

      element.insertPresetMarkdown({
        id: 'preset-1',
        name: 'first',
        markdown: 'First preset reply',
        baseDir: 'D:/presets/first',
      });
      element.insertPresetMarkdown({
        id: 'preset-2',
        name: 'second',
        markdown: 'Second preset reply',
      });
      await element.updateComplete;

      let presetContent = element.shadowRoot?.querySelectorAll(
        'agent-conversation-rich-content',
      );
      expect(presetContent).toHaveLength(2);
      expect((presetContent?.[1] as HTMLElement & { content: string }).content)
        .toBe('First preset reply');

      await vi.runAllTimersAsync();
      await element.updateComplete;

      presetContent = element.shadowRoot?.querySelectorAll(
        'agent-conversation-rich-content',
      );
      expect(Array.from(presetContent ?? []).map(
        (content) => (content as HTMLElement & { content: string }).content,
      )).toEqual(['Ready', 'First preset reply', 'Second preset reply']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('preserves insertion order for presentation items sharing an anchor', async () => {
    const element = new AgentRuntimeConversationElement();
    element.transport = new TestTransport();
    element.progressiveReveal = false;
    document.body.append(element);
    await element.connect();
    for (const [id, content] of [['one', 'One'], ['two', 'Two']] as const) {
      element.insertPresentationItem({
        contract: PRESENTATION_CONTRACT,
        id,
        kind: 'assistant-markdown',
        anchor: { type: 'after-record', recordId: 'assistant-1' },
        content,
        reveal: 'none',
      });
    }
    await element.updateComplete;

    const content = element.shadowRoot?.querySelectorAll(
      'agent-conversation-rich-content',
    );
    expect(Array.from(content ?? []).map(
      (item) => (item as HTMLElement & { content: string }).content,
    )).toEqual(['Ready', 'One', 'Two']);
  });

  it('does not replay progressive reveal for a restored waiting snapshot', async () => {
    const element = new AgentRuntimeConversationElement();
    element.transport = {
      contract: TRANSPORT_CONTRACT,
      id: 'restored-snapshot',
      async connect(_context, handlers) {
        handlers.connection('connected');
        handlers.event({
          type: 'conversation-created',
          conversationId: 'archive-1',
        });
        handlers.event({
          type: 'state-snapshot',
          conversationId: 'archive-1',
          payload: {
            revision: 7,
            conversation_state: 'waiting',
            ledger_records: [
              { record_id: 'user-1', role: 'user', content: 'hello' },
              { record_id: 'assistant-1', role: 'assistant', content: 'saved answer' },
            ],
          },
        });
        return { conversationId: 'archive-1', disconnect: vi.fn() };
      },
      async send() {
        return { accepted: true };
      },
    };
    document.body.append(element);

    await element.connect();
    await element.updateComplete;

    const content = element.shadowRoot?.querySelector(
      'agent-conversation-rich-content',
    ) as HTMLElement & { reveal: boolean };
    expect(content.reveal).toBe(false);
  });

  it('keeps persistence controls hidden unless the host enables them', async () => {
    const element = new AgentRuntimeConversationElement();
    document.body.append(element);
    await element.updateComplete;

    expect(element.shadowRoot?.querySelector('[part="persistence-actions"]')).toBeNull();
  });

  it('keeps provider controls hidden unless the host enables them', async () => {
    const element = new AgentRuntimeConversationElement();
    document.body.append(element);
    await element.updateComplete;

    expect(element.shadowRoot?.querySelector('[part="provider-actions"]')).toBeNull();
  });

  it('switches models and adds a provider without replacing existing providers', async () => {
    const controller: ConversationProviderController = {
      getProviderDefinitions: vi.fn(async () => ({
        schema: 'agent-runtime-provider-definitions/v1',
        providers: [{ uid: 1, name: 'OpenAI compatible', api_key_set: true }],
        models: [
          { uid: 1001, provider_uid: 1, model_name: 'gpt-5.1' },
          { uid: 1002, provider_uid: 1, model_name: 'deepseek-v4-flash' },
        ],
        current_model_uid: 1001,
      })),
      getBuiltinProviderCatalog: vi.fn(async () => ({
        providers: [{
          id: 'openai-gpt',
          name: 'OpenAI (GPT)',
          prefix: 'gpt-',
          defaultBaseUrl: 'https://api.openai.com/v1',
          apiFormat: 'openai',
        }],
        models: [
          {
            id: 'gpt-5.1',
            name: 'GPT-5.1',
            contextWindow: 128000,
            providerPrefix: 'gpt-',
            default: true,
          },
          {
            id: 'gpt-5.1-mini',
            name: 'GPT-5.1 Mini',
            contextWindow: 128000,
            providerPrefix: 'gpt-',
          },
        ],
      })),
      configureProviders: vi.fn(async () => ({ accepted: true })),
      setCurrentModel: vi.fn(async () => ({ accepted: true })),
    };
    const element = new AgentRuntimeConversationElement();
    element.providerControls = { enabled: true, controller };
    document.body.append(element);
    await element.updateComplete;
    await vi.waitFor(() => {
      expect(controller.getProviderDefinitions).toHaveBeenCalled();
    });

    const modelButton = Array.from(
      element.shadowRoot?.querySelectorAll<HTMLButtonElement>('.header-action') ?? [],
    ).find((button) => button.dataset.tone === 'model');
    expect(modelButton?.textContent?.trim()).toBe('gpt-5.1 / OpenAI compatible');
    modelButton?.click();
    await element.updateComplete;

    const select = element.shadowRoot?.querySelector<HTMLSelectElement>('.provider-select');
    expect(select?.textContent).toContain('deepseek-v4-flash');
    select!.value = '1002';
    select!.dispatchEvent(new Event('change'));
    await vi.waitFor(() => {
      expect(controller.setCurrentModel).toHaveBeenCalledWith({ modelUid: 1002 });
    });
    await vi.waitFor(() => {
      expect(
        Array.from(
          element.shadowRoot?.querySelectorAll<HTMLButtonElement>('.header-action') ?? [],
        ).find((button) => button.textContent?.trim() === 'Add provider')?.disabled,
      ).toBe(false);
    });

    const addButton = Array.from(
      element.shadowRoot?.querySelectorAll<HTMLButtonElement>('.header-action') ?? [],
    ).find((button) => button.textContent?.trim() === 'Add provider');
    addButton?.click();
    await element.updateComplete;

    const editor = element.shadowRoot?.querySelector<HTMLFormElement>('.provider-editor');
    expect(editor).not.toBeNull();
    await vi.waitFor(() => {
      expect(controller.getBuiltinProviderCatalog).toHaveBeenCalled();
    });
    editor!.querySelector<HTMLInputElement>('[name="provider-name"]')!.value = 'Local gateway';
    editor!.querySelector<HTMLInputElement>('[name="base-url"]')!.value = 'http://127.0.0.1:11434/v1';
    editor!.querySelector<HTMLInputElement>('[name="api-key"]')!.value = 'test-key';
    editor!.querySelector<HTMLInputElement>('[name^="model-id-"]')!.value = 'local-model';
    const addModel = element.shadowRoot?.querySelector<HTMLButtonElement>('.provider-editor-add-model');
    addModel?.click();
    await element.updateComplete;
    const modelInputs = Array.from(
      element.shadowRoot?.querySelectorAll<HTMLInputElement>('input[name^="model-id-"]') ?? [],
    );
    expect(modelInputs).toHaveLength(2);
    modelInputs[1].value = 'local-model-fast';
    editor!.dispatchEvent(new SubmitEvent('submit'));
    await vi.waitFor(() => {
      expect(controller.configureProviders).toHaveBeenCalledOnce();
    });
    const request = vi.mocked(controller.configureProviders).mock.calls[0][0];
    expect(request.source).toBe('json');
    expect(JSON.parse(request.input)).toMatchObject({
      schema: 'agent-runtime-llm-registration/v1',
      id: 'conversation-provider-editor',
      providers: [
        {
          id: 1,
          name: 'OpenAI compatible',
          type: 'openai',
          enabled_models: [
            { uid: 1001, model_id: 'gpt-5.1' },
            { uid: 1002, model_id: 'deepseek-v4-flash' },
          ],
        },
        {
          id: 2,
          name: 'Local gateway',
          type: 'openai',
          api_key: 'test-key',
          base_url: 'http://127.0.0.1:11434/v1',
          api_paradigm: 'openai_chat_completions',
          enabled_models: [{
            uid: 1003,
            model_id: 'local-model',
            max_context_tokens: 128000,
          }, {
            uid: 1004,
            model_id: 'local-model-fast',
            max_context_tokens: 128000,
          }],
        },
      ],
      current_model_uid: 1002,
    });
  });

  it('keeps persistence save as a host API instead of a default UI action', async () => {
    const element = new AgentRuntimeConversationElement();
    const controller = persistenceController();
    element.persistence = { enabled: true, controller };
    element.transport = new TestTransport();
    document.body.append(element);
    await element.connect();
    await element.updateComplete;

    const actions = Array.from(
      element.shadowRoot?.querySelectorAll<HTMLButtonElement>('.header-action') ?? [],
    ).map((button) => button.textContent?.trim());
    expect(actions).toContain('New chat');
    expect(actions).toContain('Switch');
    expect(actions).not.toContain('Save');

    await element.saveConversation();
    expect(controller.save).toHaveBeenCalledWith({
      archiveId: undefined,
      runtimeConversationId: 'conversation-1',
    });

    element.state = conversationReducer(element.state, {
      type: 'snapshot',
      payload: { revision: 2, conversation_state: 'executing' },
    });
    await element.updateComplete;
    expect(await element.saveConversation()).toBeNull();
    expect(controller.save).toHaveBeenCalledTimes(1);
  });

  it('creates a new conversation through the host controller', async () => {
    const element = new AgentRuntimeConversationElement();
    const controller = persistenceController();
    element.persistence = { enabled: true, controller };
    const open = vi.spyOn(element, 'openConversation').mockResolvedValue();
    document.body.append(element);

    await element.createConversation();

    expect(controller.create).toHaveBeenCalledOnce();
    expect(open).toHaveBeenCalledWith('runtime-new-1');
    expect(element.persistence).toMatchObject({
      enabled: true,
      binding: {
        runtimeConversationId: 'runtime-new-1',
      },
    });
  });

  it('restores an archive through the host and opens its new runtime id', async () => {
    const element = new AgentRuntimeConversationElement();
    const controller = persistenceController();
    element.persistence = { enabled: true, controller };
    const open = vi.spyOn(element, 'openConversation').mockResolvedValue();
    document.body.append(element);
    await element.restoreConversation('archive-1');

    expect(controller.restore).toHaveBeenCalledWith({ archiveId: 'archive-1' });
    expect(open).toHaveBeenCalledWith('restored-runtime-2');
    expect(element.persistence).toMatchObject({
      enabled: true,
      binding: {
        archiveId: 'archive-1',
        runtimeConversationId: 'restored-runtime-2',
      },
    });
  });
});
