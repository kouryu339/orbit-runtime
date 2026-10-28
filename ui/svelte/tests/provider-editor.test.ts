import { afterEach, describe, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Conversation from '../src/lib/Conversation.svelte';
import { AgentRuntimeConversationElement } from '../../lit/src/components/conversation-element.js';
import { AcceptanceEnvironment } from '../../playground/acceptance.js';
import type { CommandResult } from '@agent-runtime/conversation-element/host';

vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn(async () => ({
      svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    })),
  },
}));
const cleanup: Array<() => void | Promise<void>> = [];
afterEach(async () => {
  for (const dispose of cleanup.splice(0)) await dispose();
  document.body.replaceChildren();
});
async function flush() {
  await tick();
  await Promise.resolve();
  await tick();
}
function change(
  element: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement,
  value: string,
) {
  element.value = value;
  element.dispatchEvent(
    new Event(element.tagName === 'SELECT' ? 'change' : 'input', {
      bubbles: true,
    }),
  );
}
async function setup(kind: string) {
  const environment = new AcceptanceEnvironment();
  environment.scenario('empty');
  const target = document.createElement('div');
  document.body.append(target);
  const props = {
    transport: environment,
    composerController: environment.composerController,
    capabilities: environment.capabilities,
    providerControls: {
      enabled: true as const,
      controller: environment.providerController,
    },
    conversationId: 'acceptance-main',
    locale: 'zh-CN',
    theme: 'white',
    colorScheme: 'light' as const,
    progressiveReveal: false,
  };
  let root: ParentNode;
  if (kind === 'lit') {
    const element = new AgentRuntimeConversationElement();
    Object.assign(element, props);
    target.append(element);
    await element.connect();
    await element.updateComplete;
    root = element.shadowRoot!;
    cleanup.push(() => element.remove());
  } else {
    const component = mount(Conversation, { target, props });
    root = target;
    cleanup.push(() => unmount(component));
  }
  cleanup.push(() => environment.dispose());
  await flush();
  const add = Array.from(
    root.querySelectorAll<HTMLButtonElement>('.header-action'),
  ).find((button) => button.textContent?.trim() === '添加厂商')!;
  add.focus();
  add.click();
  await vi.waitFor(() =>
    expect(root.querySelector('.provider-editor')).not.toBeNull(),
  );
  await flush();
  const dialog = () => root.querySelector<HTMLElement>('.provider-editor')!;
  const submit = () =>
    dialog()
      .querySelector('form')!
      .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  const tabs = () =>
    Array.from(dialog().querySelectorAll<HTMLButtonElement>('[role="tab"]'));
  return { root, environment, add, dialog, submit, tabs };
}
describe.each(['lit', 'svelte'])('%s provider editor', (kind) => {
  it('keeps focus inside, supports keyboard mode switching and restores focus on Escape', async () => {
    const { root, add, dialog, tabs } = await setup(kind);
    expect(dialog().getAttribute('role')).toBe('dialog');
    expect(root.querySelector<HTMLElement>('.header')?.inert).toBe(true);
    expect(dialog().querySelector('.provider-editor-header')).not.toBeNull();
    expect(dialog().querySelector('.provider-editor-actions')).not.toBeNull();
    tabs()[0].dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }),
    );
    await flush();
    expect(tabs()[1].getAttribute('aria-selected')).toBe('true');
    expect(
      dialog().querySelector<HTMLFieldSetElement>('fieldset')?.disabled,
    ).toBe(true);
    const last = dialog().querySelector<HTMLButtonElement>('[type="submit"]')!;
    last.focus();
    last.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Tab',
        bubbles: true,
        cancelable: true,
      }),
    );
    const scope = dialog().getRootNode() as Document | ShadowRoot;
    expect(scope.activeElement).toBe(
      dialog().querySelector('.provider-editor-close'),
    );
    dialog().dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Escape',
        bubbles: true,
        cancelable: true,
      }),
    );
    await flush();
    expect(root.querySelector('.provider-editor')).toBeNull();
    expect(root.querySelector<HTMLElement>('.header')?.inert).toBe(false);
    expect(scope.activeElement).toBe(add);
  });
  it('loads presets, rejects duplicate models and saves without replacing existing models', async () => {
    const { dialog, environment, submit } = await setup(kind);
    const configure = vi.spyOn(
      environment.providerController,
      'configureProviders',
    );
    await vi.waitFor(() =>
      expect(
        dialog().querySelector<HTMLSelectElement>('[name="builtin-provider"]')!
          .options.length,
      ).toBeGreaterThan(1),
    );
    change(
      dialog().querySelector<HTMLSelectElement>('[name="builtin-provider"]')!,
      'openai-gpt',
    );
    await flush();
    expect(
      dialog().querySelector<HTMLInputElement>('[name^="model-id-"]')!.value,
    ).toBe('chat-demo');
    dialog()
      .querySelector<HTMLButtonElement>('.provider-editor-add-model')!
      .click();
    await flush();
    let rows = Array.from(
      dialog().querySelectorAll<HTMLInputElement>('[name^="model-id-"]'),
    );
    change(rows[1], 'chat-demo');
    await flush();
    submit();
    await flush();
    expect(configure).not.toHaveBeenCalled();
    expect(dialog().querySelector('[role="alert"]')?.textContent).toContain(
      '不能重复',
    );
    change(rows[1], 'second-model');
    await flush();
    submit();
    await vi.waitFor(() => expect(configure).toHaveBeenCalledOnce());
    const request = JSON.parse(configure.mock.calls[0][0].input);
    expect(request.providers).toHaveLength(4);
    expect(request.current_model_uid).toBe(1001);
    expect(request.providers[0].enabled_models[0].reasoning).toEqual({
      effort: 'medium',
    });
    expect(
      request.providers
        .at(-1)
        .enabled_models.map((m: { model_id: string }) => m.model_id),
    ).toEqual(['chat-demo', 'second-model']);
  });
  it('validates import JSON, blocks duplicate submits and retains the candidate after rejection', async () => {
    const { dialog, root, environment, submit, tabs } = await setup(kind);
    tabs()[1].click();
    await flush();
    const configure = vi.spyOn(
      environment.providerController,
      'configureProviders',
    );
    const json = dialog().querySelector<HTMLTextAreaElement>(
      '[aria-label="Provider JSON"]',
    )!;
    change(json, '{invalid');
    await flush();
    submit();
    await flush();
    expect(configure).not.toHaveBeenCalled();
    expect(dialog().querySelector('[role="alert"]')?.textContent).toContain(
      'JSON',
    );
    const candidate =
      '{"schema":"agent-runtime-llm-registration/v1","providers":[]}';
    change(json, candidate);
    await flush();
    let finish!: (result: CommandResult) => void;
    configure.mockImplementationOnce(
      () => new Promise((resolve) => (finish = resolve)),
    );
    submit();
    submit();
    await flush();
    expect(configure).toHaveBeenCalledOnce();
    expect(
      dialog().querySelector<HTMLButtonElement>('.provider-editor-close')
        ?.disabled,
    ).toBe(true);
    expect(tabs().every((button) => button.disabled)).toBe(true);
    dialog().dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Escape',
        bubbles: true,
        cancelable: true,
      }),
    );
    await flush();
    expect(root.querySelector('.provider-editor')).not.toBeNull();
    finish({ accepted: false, rejectReason: 'fixture rejected' });
    await flush();
    expect(json.value).toBe(candidate);
    expect(dialog().querySelector('[role="alert"]')?.textContent).toContain(
      'fixture rejected',
    );
    configure.mockResolvedValueOnce({ accepted: true });
    submit();
    await vi.waitFor(() =>
      expect(root.querySelector('.provider-editor')).toBeNull(),
    );
    expect(configure).toHaveBeenCalledTimes(2);
  });
  it('loads files as editable candidates without importing automatically', async () => {
    const { dialog, environment, tabs } = await setup(kind);
    tabs()[1].click();
    await flush();
    const configure = vi.spyOn(
      environment.providerController,
      'configureProviders',
    );
    const input = dialog().querySelector<HTMLInputElement>('[type="file"]')!;
    Object.defineProperty(input, 'files', {
      configurable: true,
      value: [{ text: async () => '{"providers":[]}' }],
    });
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    expect(
      dialog().querySelector<HTMLTextAreaElement>(
        '[aria-label="Provider JSON"]',
      )?.value,
    ).toBe('{"providers":[]}');
    expect(configure).not.toHaveBeenCalled();
  });
});
