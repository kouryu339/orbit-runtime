import { afterEach, describe, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Conversation from '../src/lib/Conversation.svelte';
import { AgentRuntimeConversationElement } from '../../lit/src/components/conversation-element.js';
import { AcceptanceEnvironment } from '../../playground/acceptance.js';

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
  vi.useRealTimers();
});
async function flush() {
  await tick();
  await Promise.resolve();
  await tick();
}
async function setup(kind: string) {
  const environment = new AcceptanceEnvironment();
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
    progressiveReveal: false,
  };
  let root: ParentNode;
  let api: { disconnect(): void };
  if (kind === 'lit') {
    const element = new AgentRuntimeConversationElement();
    Object.assign(element, props);
    target.append(element);
    await element.connect();
    await element.updateComplete;
    root = element.shadowRoot!;
    api = element;
    cleanup.push(() => element.remove());
  } else {
    const component = mount(Conversation, { target, props });
    api = component;
    cleanup.push(() => unmount(component));
    root = target;
  }
  cleanup.push(() => environment.dispose());
  await flush();
  await vi.waitFor(() =>
    expect(
      root.querySelector<HTMLButtonElement>(
        '[aria-label="Model and reasoning"]',
      )?.disabled,
    ).toBe(false),
  );
  return { root, environment, target, api };
}
describe.each(['lit', 'svelte'])('%s composer', (kind) => {
  it('places images and permissions on the left and switches per-conversation reasoning and models on the right', async () => {
    const { root, environment } = await setup(kind);
    expect(
      root.querySelector('.composer-left [aria-label="Attach images"]'),
    ).not.toBeNull();
    expect(
      root.querySelector('.composer-left [aria-label="Permissions"]'),
    ).not.toBeNull();
    expect(root.querySelectorAll('.primary-action')).toHaveLength(1);
    const modelButton = root.querySelector<HTMLButtonElement>(
      '[aria-label="Model and reasoning"]',
    )!;
    modelButton.click();
    await flush();
    const slider = root.querySelector<HTMLInputElement>(
      '[aria-label="Reasoning effort"]',
    )!;
    expect(slider.disabled).toBe(false);
    const setEffort = vi.spyOn(environment.composerController, 'setEffort');
    slider.value = '1';
    slider.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(root.querySelector('.reasoning-label')?.textContent).toContain(
      '最低',
    );
    expect(slider.getAttribute('aria-valuetext')).toBe('最低');
    slider.value = '3';
    slider.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(root.querySelector('.reasoning-label')?.textContent).toContain('中');
    expect(slider.value).toBe('3');
    expect(setEffort).not.toHaveBeenCalled();
    slider.value = '4';
    slider.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() =>
      expect(root.querySelector('.effort-value')?.textContent).toBe('高'),
    );
    expect(setEffort).toHaveBeenCalledOnce();
    expect(root.querySelector('.header-title')?.textContent).toBe('对话');
    expect(root.querySelector('.connection-label')?.textContent).toBe('已连接');
    expect(root.querySelector('.status-dot')).toBeNull();
    const definitions =
      await environment.providerController.getProviderDefinitions();
    expect(definitions.models?.[0].reasoning?.effort).toBe('medium');
    const modelItems = Array.from(
      root.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'),
    );
    modelItems
      .find((button) => button.textContent?.includes('claude-demo'))!
      .click();
    await vi.waitFor(() =>
      expect(root.querySelector('.model-name')?.textContent).toBe(
        'claude-demo',
      ),
    );
    await vi.waitFor(() =>
      expect(
        root.querySelector<HTMLInputElement>('[aria-label="Reasoning effort"]')
          ?.max,
      ).toBe('2'),
    );
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await flush();
    expect(root.querySelector('.composer-menu')).toBeNull();
    root
      .querySelector<HTMLButtonElement>('[aria-label="Permissions"]')!
      .click();
    await flush();
    Array.from(
      root.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'),
    )
      .find((button) => button.textContent?.includes('帮我批准'))!
      .click();
    await vi.waitFor(() =>
      expect(
        root.querySelector('[aria-label="Permissions"]')?.textContent,
      ).toContain('帮我批准'),
    );
  });
  it('transitions one button through pause and continue and blocks offline and compacting actions', async () => {
    const { root, environment, api } = await setup(kind);
    const pause = vi.spyOn(environment, 'pause');
    const resume = vi.spyOn(environment, 'resume');
    const send = vi.spyOn(environment, 'send');
    environment.scenario('stream');
    await flush();
    const pauseButton = root.querySelector<HTMLButtonElement>(
      '[aria-label="Pause response"]',
    )!;
    expect(pauseButton.disabled).toBe(false);
    pauseButton.click();
    pauseButton.click();
    await flush();
    expect(pause).toHaveBeenCalledOnce();
    await vi.waitFor(() =>
      expect(
        root.querySelector<HTMLButtonElement>(
          '[aria-label="Continue response"]',
        )?.disabled,
      ).toBe(false),
    );
    root
      .querySelector<HTMLButtonElement>('[aria-label="Continue response"]')!
      .click();
    await flush();
    expect(resume).toHaveBeenCalledOnce();
    expect(send).not.toHaveBeenCalled();
    expect(
      root.querySelector('.primary-action')?.getAttribute('data-state'),
    ).toBe('thinking');
    environment.compact();
    await flush();
    expect(
      root.querySelector<HTMLButtonElement>('.primary-action')?.disabled,
    ).toBe(true);
    expect(
      root.querySelector('.primary-action')?.getAttribute('data-state'),
    ).toBe('compacting');
    api.disconnect();
    await flush();
    expect(
      root.querySelector('.primary-action')?.getAttribute('data-state'),
    ).toBe('offline');
    expect(
      root.querySelector<HTMLButtonElement>('.primary-action')?.disabled,
    ).toBe(true);
    expect(
      root.querySelector<HTMLButtonElement>(
        '[aria-label="Model and reasoning"]',
      )?.disabled,
    ).toBe(true);
  });
  it('keeps approval actions explicit and prevents sending or changing settings while approval is pending', async () => {
    const { root, environment } = await setup(kind);
    environment.scenario('approval');
    await flush();
    expect(
      root.querySelector('.primary-action')?.getAttribute('data-state'),
    ).toBe('approval');
    expect(root.querySelector<HTMLTextAreaElement>('textarea')?.disabled).toBe(
      true,
    );
    expect(
      root.querySelector<HTMLButtonElement>(
        '[aria-label="Model and reasoning"]',
      )?.disabled,
    ).toBe(true);
    expect(root.querySelectorAll('.permission-tool')).toHaveLength(2);
    root
      .querySelector<HTMLButtonElement>('.permission-actions button')!
      .click();
    await flush();
    await vi.waitFor(() =>
      expect(root.querySelectorAll('.permission-tool')).toHaveLength(1),
    );
    expect(root.querySelector<HTMLTextAreaElement>('textarea')?.disabled).toBe(
      true,
    );
  });
});
