import { afterEach, describe, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Conversation from '../src/lib/Conversation.svelte';
import ProviderEditor from '../src/lib/ProviderEditor.svelte';
import RichContent from '../src/lib/RichContent.svelte';
import parity from '../../feature-parity.json';
import { AgentRuntimeConversationElement } from '../../lit/src/components/conversation-element.js';
import {
  TRANSPORT_CONTRACT,
  PERSISTENCE_CONTRACT,
  type ConversationTransport,
  type ConversationTransportHandlers,
  type SendMessageRequest,
  type CommandResult,
  type ConversationPersistenceEvent,
  type ConversationProviderController,
} from '@agent-runtime/conversation-element/host';

const instances: ReturnType<typeof mount>[] = [];
class Transport implements ConversationTransport {
  contract = TRANSPORT_CONTRACT;
  id = 'parity';
  handlers: ConversationTransportHandlers[] = [];
  imageInput = {
    importImage: vi.fn(async ({ file }: { file: File }) => ({
      imageId: `asset:${file.name}`,
    })),
  };
  send = vi.fn(
    async (_request: SendMessageRequest): Promise<CommandResult> => ({
      accepted: true,
    }),
  );
  requestSnapshot = vi.fn(async () => {});
  close = vi.fn(async (): Promise<CommandResult> => ({ accepted: true }));
  resolveToolPermission = vi.fn(
    async (): Promise<CommandResult> => ({ accepted: true }),
  );
  async connect(
    context: { conversationId?: string | null },
    handlers: ConversationTransportHandlers,
  ) {
    this.handlers.push(handlers);
    handlers.connection('connected');
    handlers.event({
      type: 'conversation-created',
      conversationId: context.conversationId ?? 'one',
    });
    return { disconnect: vi.fn() };
  }
  snapshot(
    payload: Record<string, unknown> = {},
    index = this.handlers.length - 1,
    id = 'one',
  ) {
    this.handlers[index].event({
      type: 'state-snapshot',
      conversationId: id,
      payload: {
        conversation_state: 'waiting',
        ledger_records: [],
        ...payload,
      },
    });
  }
}
async function setup(
  props: Record<string, unknown> = {},
  events: Record<string, (event: CustomEvent<any>) => void> = {},
) {
  const target = document.createElement('div');
  document.body.append(target);
  const transport = new Transport();
  const component = mount(Conversation, {
    target,
    props: { transport, locale: 'zh-CN', ...props },
    events,
  });
  instances.push(component);
  await tick();
  await Promise.resolve();
  await tick();
  return { component, transport, target };
}
async function flush() {
  await tick();
  await Promise.resolve();
  await tick();
}
function change(
  element: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement,
  value: string,
) {
  element.value = value;
  element.dispatchEvent(
    new Event(element.tagName === 'SELECT' ? 'change' : 'input', {
      bubbles: true,
    }),
  );
}
async function attach(target: HTMLElement, files: File[]) {
  const input = target.querySelector<HTMLInputElement>('input.image-input')!;
  Object.defineProperty(input, 'files', { configurable: true, value: files });
  input.dispatchEvent(new Event('change', { bubbles: true }));
  await flush();
}
afterEach(async () => {
  for (const instance of instances.splice(0)) await unmount(instance);
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe('Lit and Svelte parity', () => {
  it('resolves Svelte image thumbnails and ignores stale results after a new reference', async () => {
    const pending = new Map<string, (value: { source: string }) => void>();
    const openImage = vi.fn();
    const { transport, target } = await setup({
      capabilities: {
        resolveImage: ({ imageId }: { imageId: string }) =>
          new Promise((resolve) => pending.set(imageId, resolve)),
        openImage,
      },
    });
    const show = (id: string) =>
      transport.snapshot({
        ledger_records: [
          {
            record_id: 'user-image',
            role: 'user',
            content: '',
            metadata: { extra: { parts: [{ type: 'image', image_id: id }] } },
          },
        ],
      });
    show('old');
    await flush();
    show('new');
    await flush();
    pending.get('new')!({ source: 'https://example.com/new.png' });
    await flush();
    pending.get('old')!({ source: 'https://example.com/old.png' });
    await flush();
    const img = target.querySelector<HTMLImageElement>(
      '.orbit-svelte-image-reference img',
    )!;
    expect(img.src).toBe('https://example.com/new.png');
    target
      .querySelector<HTMLButtonElement>('.orbit-svelte-image-reference button')!
      .click();
    expect(openImage).toHaveBeenCalledWith(
      expect.objectContaining({
        source: img.src,
        metadata: { imageId: 'new' },
      }),
    );
    img.dispatchEvent(new Event('error'));
    await flush();
    expect(
      target.querySelector('.orbit-svelte-image-reference img'),
    ).toBeNull();
    expect(target.textContent).toContain('@图片1');
  });
  it('exposes every shared public method and keeps host-only actions hidden', async () => {
    const { component, target } = await setup();
    for (const method of parity.public_api)
      expect(typeof component[method]).toBe('function');
    expect(target.querySelector('[part="persistence-actions"]')).toBeNull();
    expect(target.querySelector('[part="provider-actions"]')).toBeNull();
    expect(component.locale).toBe('zh-CN');
  });
  it('updates the model and activity labels in place and switches themes without reconnecting', async () => {
    const controller = {
      getProviderDefinitions: vi.fn(async () => ({
        providers: [{ uid: 1, name: 'Provider' }],
        models: [{ uid: 10, provider_uid: 1, model_id: 'Model' }],
      })),
      configureProviders: vi.fn(async () => ({ accepted: true })),
      setConversationModel: vi.fn(async () => ({ accepted: true })),
    };
    const { component, transport, target } = await setup({
      providerControls: { enabled: true, controller },
    });
    transport.snapshot({ conversation_state: 'thinking' });
    await flush();
    expect(target.textContent).toContain('Loading');
    transport.snapshot({ conversation_state: 'compacting' });
    await flush();
    expect(target.textContent).toContain('总结中');
    transport.snapshot();
    await flush();
    await component.setConversationModel(10);
    await flush();
    expect(target.querySelector('[data-tone="model"]')?.textContent).toContain(
      'Model',
    );
    component.theme = 'paper';
    await flush();
    expect(target.querySelector('[data-theme="paper"]')).not.toBeNull();
    expect(transport.handlers).toHaveLength(1);
  });
  it('does not close or restore over a conversation opened while the request was pending', async () => {
    let completeRestore!: (binding: {
      runtimeConversationId: string;
      archiveId: string;
    }) => void;
    const controller = {
      contract: PERSISTENCE_CONTRACT,
      list: vi.fn(async () => ({ items: [] })),
      save: vi.fn(async () => ({
        archiveId: 'a',
        runtimeConversationId: 'one',
        status: 'sealed' as const,
      })),
      restore: vi.fn(
        () =>
          new Promise<{ runtimeConversationId: string; archiveId: string }>(
            (resolve) => (completeRestore = resolve),
          ),
      ),
    };
    const { component, transport } = await setup({
      persistence: { enabled: true, controller },
    });
    transport.snapshot();
    await flush();
    const restoring = component.restoreConversation('old');
    await component.openConversation('two');
    transport.snapshot({}, 1, 'two');
    completeRestore({
      archiveId: 'old',
      runtimeConversationId: 'old-restored',
    });
    await restoring;
    expect(component.state.conversationId).toBe('two');
    let completeClose!: (value: CommandResult) => void;
    transport.close.mockImplementationOnce(
      () => new Promise((resolve) => (completeClose = resolve)),
    );
    const closing = component.closeConversation();
    await component.openConversation('three');
    transport.snapshot({}, 2, 'three');
    completeClose({ accepted: true });
    await closing;
    expect(component.state.conversationId).toBe('three');
  });
  it('projects identical user, assistant, gateway error and tool records', async () => {
    const payload = {
      ledger_records: [
        {
          record_id: 'u',
          role: 'user',
          content: 'Hello @alice',
          metadata: {
            extra: {
              parts: [
                { type: 'text', text: 'Hello @alice ' },
                { type: 'image', image_id: 'image-1' },
              ],
            },
          },
        },
        {
          record_id: 'a',
          role: 'assistant',
          content: 'Reading',
          metadata: {
            extra: {
              tool_calls: [{ id: 'call-1', function: { name: 'Read' } }],
            },
          },
        },
        {
          record_id: 't',
          role: 'gateway_message',
          content: 'Done',
          metadata: {
            subtype: 'tool_call_finished',
            tool_name: 'Read',
            extra: { call_id: 'call-1' },
          },
        },
        {
          record_id: 'e',
          role: 'gateway_message',
          content: 'Provider unavailable',
          metadata: { subtype: 'llm_error' },
        },
        {
          record_id: 'noise',
          role: 'gateway_message',
          content: 'internal-only',
          metadata: { subtype: 'trace' },
        },
      ],
    };
    const { transport, target } = await setup();
    transport.snapshot(payload);
    await flush();
    const lit = new AgentRuntimeConversationElement();
    lit.transport = new Transport();
    lit.locale = 'zh-CN';
    document.body.append(lit);
    await lit.connect();
    (lit.transport as Transport).snapshot(payload);
    await lit.updateComplete;
    const root = lit.shadowRoot!;
    expect(target.querySelectorAll('article.message')).toHaveLength(
      root.querySelectorAll('article.message').length,
    );
    expect(target.querySelectorAll('.at-reference').length).toBe(
      root.querySelectorAll('.at-reference').length,
    );
    for (const text of ['@alice', '@图片1', 'Provider unavailable']) {
      expect(target.textContent).toContain(text);
      expect(root.textContent).toContain(
        text === 'Provider unavailable' ? '' : text,
      );
    }
    expect(target.textContent).not.toContain('internal-only');
    expect(target.querySelector('details.tool')).not.toBeNull();
    expect(
      target
        .querySelector<HTMLElement>('[data-reference-key="image:image-1"]')
        ?.style.getPropertyValue('--reference-color'),
    ).toBe(
      root
        .querySelector<HTMLElement>('[data-reference-key="image:image-1"]')
        ?.style.getPropertyValue('--reference-color'),
    );
    lit.remove();
  });
  it('uploads images and preserves ordered image/text parts and public send events', async () => {
    const sent = vi.fn();
    const { transport, target, component } = await setup(
      {},
      { 'agent-conversation-send': sent },
    );
    transport.snapshot();
    await flush();
    await attach(target, [
      new File(['one'], 'one.png', { type: 'image/png' }),
      new File(['two'], 'two.jpg', { type: 'image/jpeg' }),
    ]);
    const composer = target.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Message"]',
    )!;
    change(composer, 'before @图片2 between @图片1 after');
    await flush();
    expect((await component.send(composer.value)).accepted).toBe(true);
    await flush();
    expect(transport.send.mock.calls[0][0].parts).toEqual([
      { type: 'text', text: 'before ' },
      { type: 'image', image_id: 'asset:two.jpg' },
      { type: 'text', text: ' between ' },
      { type: 'image', image_id: 'asset:one.png' },
      { type: 'text', text: ' after' },
    ]);
    expect(sent).toHaveBeenCalledOnce();
    expect(target.querySelectorAll('.image-attachment')).toHaveLength(0);
    expect(composer.value).toBe('');
  });
  it('supports image-only sends and revokes previews when deleting or switching', async () => {
    const revoke = vi.fn();
    Object.defineProperty(URL, 'createObjectURL', {
      configurable: true,
      value: vi.fn(() => `blob:preview`),
    });
    Object.defineProperty(URL, 'revokeObjectURL', {
      configurable: true,
      value: revoke,
    });
    const { transport, target, component } = await setup();
    transport.snapshot();
    await flush();
    await attach(target, [new File(['x'], 'x.png', { type: 'image/png' })]);
    await component.send('');
    expect(transport.send.mock.calls[0][0].content).toBe('[Image]');
    expect(revoke).toHaveBeenCalled();
    transport.snapshot();
    await flush();
    await attach(target, [new File(['x'], 'x.png', { type: 'image/png' })]);
    await component.openConversation('two');
    expect(revoke).toHaveBeenCalledTimes(2);
  });
  it('rejects invalid images and retains attachments after upload failure', async () => {
    const errors = vi.fn();
    const { transport, target, component } = await setup(
      {},
      { 'agent-conversation-error': errors },
    );
    transport.snapshot();
    await flush();
    await attach(target, [
      new File(['pdf'], 'bad.pdf', { type: 'application/pdf' }),
      new File(['ok'], 'ok.png', { type: 'image/png' }),
    ]);
    expect(target.querySelectorAll('.image-attachment')).toHaveLength(1);
    transport.imageInput.importImage.mockRejectedValueOnce(
      new Error('upload failed'),
    );
    expect((await component.send('retry @图片1')).accepted).toBe(false);
    await flush();
    expect(transport.send).not.toHaveBeenCalled();
    expect(target.querySelectorAll('.image-attachment')).toHaveLength(1);
    expect(errors).toHaveBeenCalled();
  });
  it('does not deliver an imported image to a newly opened conversation', async () => {
    const { transport, target, component } = await setup();
    transport.snapshot();
    await flush();
    await attach(target, [new File(['x'], 'x.png', { type: 'image/png' })]);
    let finish!: (value: { imageId: string }) => void;
    transport.imageInput.importImage.mockImplementationOnce(
      () => new Promise((resolve) => (finish = resolve)),
    );
    const send = component.send('look @图片1');
    await component.openConversation('two');
    transport.snapshot({}, 1, 'two');
    finish({ imageId: 'old' });
    expect((await send).accepted).toBe(false);
    expect(transport.send).not.toHaveBeenCalled();
  });
  it('renumbers image markers after deleting an attachment and handles pasted images', async () => {
    const { transport, target } = await setup();
    transport.snapshot();
    await flush();
    const composer = target.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Message"]',
    )!;
    const paste = new Event('paste', { bubbles: true, cancelable: true });
    Object.defineProperty(paste, 'clipboardData', {
      value: {
        items: [
          {
            kind: 'file',
            getAsFile: () =>
              new File(['x'], 'pasted.png', { type: 'image/png' }),
          },
        ],
        getData: () => 'context',
      },
    });
    composer.dispatchEvent(paste);
    await flush();
    expect(composer.value).toContain('context @图片1');
    await attach(target, [new File(['x'], 'next.png', { type: 'image/png' })]);
    target
      .querySelector<HTMLButtonElement>('[aria-label="Remove pasted.png"]')!
      .click();
    await flush();
    expect(composer.value).not.toContain('@图片2');
    expect(composer.value).toContain('@图片1');
  });
  it('clears terminal approvals and accepts permission-resolved extensions without registration', async () => {
    const { transport, target, component } = await setup();
    const permission = {
      tool_call_id: 'call-1',
      tool_name: 'Read',
      effect: 'read_only',
    };
    transport.snapshot({ pending_permissions: [permission] });
    await flush();
    transport.resolveToolPermission.mockResolvedValueOnce({
      accepted: false,
      metadata: { resolved: false },
    });
    await component.resolveToolPermission('call-1', 'allow');
    await flush();
    expect(target.querySelector('.approval')).toBeNull();
    expect(transport.requestSnapshot).toHaveBeenCalledWith({
      conversationId: 'one',
    });
    transport.snapshot({
      pending_permissions: [{ ...permission, tool_call_id: 'call-2' }],
    });
    await flush();
    transport.handlers.at(-1)!.event({
      type: 'transport-extension',
      extension: {
        contract: 'agent-conversation-extension/v1',
        namespace: 'runtime',
        version: 1,
        kind: 'tool:permission-resolved',
        payload: { tool_call_id: 'call-2' },
      },
    });
    await flush();
    expect(target.querySelector('.approval')).toBeNull();
  });
  it('updates one live plan, preserves collapse, and escapes step text', async () => {
    const { transport, target } = await setup();
    const plan = {
      plan_id: 'p',
      revision: 1,
      title: 'Plan',
      status: 'active',
      steps: [
        { id: 'a', text: '<img src=x onerror=bad()>', status: 'in_progress' },
      ],
    };
    transport.snapshot({ plan });
    await flush();
    const details = target.querySelector<HTMLDetailsElement>(
      '.orbit-svelte-plan details',
    )!;
    expect(details.open).toBe(true);
    details.open = false;
    details.dispatchEvent(new Event('toggle'));
    transport.snapshot({
      plan: {
        ...plan,
        revision: 2,
        status: 'finished',
        steps: [{ ...plan.steps[0], status: 'completed' }],
      },
    });
    await flush();
    expect(target.querySelectorAll('.orbit-svelte-plan')).toHaveLength(1);
    expect(details.open).toBe(false);
    expect(target.querySelector('.orbit-svelte-plan img')).toBeNull();
    expect(target.querySelector('progress')?.value).toBe(1);
    transport.snapshot({ plan: null });
    await flush();
    expect(target.querySelector('.orbit-svelte-plan')).toBeNull();
  });
  it('expires an older widget when the newest message is from the user', async () => {
    const { transport, target } = await setup();
    transport.snapshot({
      ledger_records: [
        {
          record_id: 'a',
          role: 'assistant',
          content: '[select:single | label="Choose" | options="A,B"]',
        },
        { record_id: 'u', role: 'user', content: 'next' },
      ],
    });
    await flush();
    expect(
      target.querySelector<HTMLInputElement>('.widgets input')?.disabled,
    ).toBe(true);
    expect(target.textContent).toContain('已过期');
  });
  it('keeps history subscription state and isolates stale provider results', async () => {
    let notify!: (event: ConversationPersistenceEvent) => void;
    const unsubscribe = vi.fn();
    const persistence = {
      enabled: true,
      controller: {
        contract: PERSISTENCE_CONTRACT,
        list: vi.fn(async () => ({ items: [] })),
        save: vi.fn(async () => ({
          archiveId: 'saved',
          runtimeConversationId: 'one',
          status: 'sealed' as const,
        })),
        restore: vi.fn(async () => ({
          archiveId: 'old',
          runtimeConversationId: 'two',
        })),
        subscribe: (listener: typeof notify) => {
          notify = listener;
          return unsubscribe;
        },
      },
    };
    const { transport, target, component } = await setup({ persistence });
    transport.snapshot();
    await flush();
    notify({
      type: 'archive-updated',
      archive: {
        archiveId: 'old',
        runtimeConversationId: null,
        title: 'Stored',
        instanceState: 'background',
        status: 'sealed',
      },
    });
    const toggle = [...target.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === '历史会话',
    )!;
    toggle.click();
    await flush();
    // A refreshed list is authoritative; then a live update repopulates it.
    notify({
      type: 'archive-updated',
      archive: {
        archiveId: 'old',
        runtimeConversationId: null,
        title: 'Stored',
        instanceState: 'background',
        status: 'sealed',
      },
    });
    await flush();
    expect(target.textContent).toContain('后台运行');
    await component.restoreConversation('old');
    expect(component.state.conversationId).toBe('two');
    notify({ type: 'archive-deleted', archiveId: 'old' });
    await flush();
    expect(unsubscribe).not.toHaveBeenCalled();
  });
  it('emits connection and provider errors, rejects model selection without changing state', async () => {
    const errors = vi.fn(),
      connections = vi.fn();
    const controller = {
      getProviderDefinitions: vi.fn(async () => ({
        providers: [{ uid: 1, name: 'Provider' }],
        models: [{ uid: 10, provider_uid: 1, model_id: 'Model' }],
      })),
      configureProviders: vi.fn(async () => ({ accepted: true })),
      setConversationModel: vi.fn(async () => ({
        accepted: false,
        rejectReason: 'busy',
      })),
    };
    const { component, transport } = await setup(
      { providerControls: { enabled: true, controller } },
      {
        'agent-conversation-error': errors,
        'agent-conversation-connection-change': connections,
      },
    );
    transport.snapshot();
    await flush();
    await component.setConversationModel(10);
    expect(component.state.modelUid).toBeUndefined();
    expect(errors.mock.calls.at(-1)![0].detail.code).toBe(
      'provider-model-failed',
    );
    expect(connections).toHaveBeenCalled();
  });
});

describe('Svelte model reasoning editor', () => {
  const definitions = {
    current_model_uid: 10,
    providers: [
      {
        uid: 1,
        name: 'One',
        base_url: 'https://one.invalid/v1',
        api_paradigm: 'openai_responses',
      },
      {
        uid: 2,
        name: 'Two',
        base_url: 'https://two.invalid/v1',
        api_paradigm: 'anthropic_messages',
      },
    ],
    models: [
      {
        uid: 10,
        provider_uid: 1,
        model_id: 'one',
        reasoning: { effort: 'low' as const, summary: 'auto' as const },
      },
      {
        uid: 20,
        provider_uid: 2,
        model_id: 'two',
        reasoning: { effort: 'high' as const, thinking: 'adaptive' as const },
      },
    ],
  };
  async function editor() {
    const target = document.createElement('div');
    document.body.append(target);
    const controller: ConversationProviderController = {
      getProviderDefinitions: vi.fn(async () => definitions),
      configureProviders: vi.fn(async () => ({ accepted: true })),
      setConversationModel: vi.fn(async () => ({ accepted: true })),
    };
    const component = mount(ProviderEditor, {
      target,
      props: { controller, definitions, providerUid: 1 },
    });
    instances.push(component);
    await flush();
    return { controller, target };
  }
  it('edits reasoning without losing other models, reasoning or the selected model', async () => {
    const { target, controller } = await editor();
    change(
      target.querySelector<HTMLSelectElement>('[name="model-effort-0"]')!,
      'high',
    );
    await flush();
    target
      .querySelector('form')!
      .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();
    const request = JSON.parse(
      vi.mocked(controller.configureProviders).mock.calls[0][0].input,
    );
    expect(request.current_model_uid).toBe(10);
    expect(
      request.providers.find((p: any) => p.id === 1).enabled_models[0]
        .reasoning,
    ).toEqual({ effort: 'high', summary: 'auto' });
    expect(
      request.providers.find((p: any) => p.id === 2).enabled_models[0]
        .reasoning,
    ).toEqual(definitions.models[1].reasoning);
  });
  it('omits default reasoning and incompatible fields after switching paradigm', async () => {
    const { target, controller } = await editor();
    change(
      target.querySelector<HTMLSelectElement>('[name="api-paradigm"]')!,
      'openai_chat_completions',
    );
    await flush();
    const select = target.querySelector<HTMLSelectElement>(
      '[name="model-effort-0"]',
    )!;
    select.selectedIndex = 0;
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    target
      .querySelector('form')!
      .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();
    const request = JSON.parse(
      vi.mocked(controller.configureProviders).mock.calls[0][0].input,
    );
    expect(
      request.providers.find((p: any) => p.id === 1).enabled_models[0],
    ).not.toHaveProperty('reasoning');
  });
});

describe('Svelte rich content', () => {
  async function content(
    text: string,
    props: Record<string, unknown> = {},
    events: Record<string, (event: CustomEvent<any>) => void> = {},
  ) {
    const target = document.createElement('div');
    document.body.append(target);
    const instance = mount(RichContent, {
      target,
      props: { content: text, ...props },
      events,
    });
    instances.push(instance);
    await flush();
    return target;
  }
  it('aggregates widget groups once, supports Other and confirm cancel, prevents resubmission', async () => {
    const submit = vi.fn();
    const target = await content(
      '[select:single | label="Choice" | options="A,B"]\n\nDetails\n\n[confirm | label="Continue"]',
      {},
      { submit },
    );
    const other = target.querySelector<HTMLInputElement>('.other-input')!;
    change(other, 'Custom');
    await flush();
    const cancel = [...target.querySelectorAll('button')].find(
      (b) => b.textContent === 'Cancel',
    )!;
    cancel.click();
    await flush();
    const button = [...target.querySelectorAll('button')].find(
      (b) => b.textContent === 'Submit',
    )!;
    button.click();
    await flush();
    expect(submit).toHaveBeenCalledOnce();
    expect(submit.mock.calls[0][0].detail.content).toBe(
      'Choice: Custom\nContinue: no',
    );
    expect(target.textContent).toContain('Submitted');
  });
  it('passes image metadata and safe links to the host, renders formulas and sanitizes HTML', async () => {
    const openLink = vi.fn(async () => {}),
      openImage = vi.fn(async () => {});
    const target = await content(
      '[Safe](https://example.invalid)\n\n![Image](relative.png)\n\n$x^2$\n\n<img src=x onerror=bad()>',
      {
        capabilities: { openLink, openImage },
        baseDir: 'assets',
        contentMetadata: { asset: 'a' },
      },
    );
    target.querySelector<HTMLAnchorElement>('a')!.click();
    target.querySelector<HTMLImageElement>('img')!.click();
    await flush();
    expect(openLink).toHaveBeenCalledWith({
      url: 'https://example.invalid/',
      source: 'markdown',
    });
    expect(openImage).toHaveBeenCalledWith({
      source: 'relative.png',
      alt: 'Image',
      baseDir: 'assets',
      metadata: { asset: 'a' },
    });
    expect(target.querySelector('[onerror]')).toBeNull();
    expect(target.querySelector('math')).not.toBeNull();
  });
});
