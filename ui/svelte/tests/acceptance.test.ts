import { afterEach, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import App from '../src/App.svelte';
import { AgentRuntimeConversationElement } from '../../lit/src/components/conversation-element.js';
import { parseConversationContent } from '@agent-runtime/conversation-element/content';
import {
  AcceptanceEnvironment,
  scenarios,
} from '../../playground/acceptance.js';
import { ImageReferenceElement } from '../../lit/src/components/image-reference.js';
import { ConversationRichContentElement } from '../../lit/src/components/rich-content.js';
import { isSafeImageSource } from '@agent-runtime/conversation-element/content';
import type { ConversationTransportEvent } from '@agent-runtime/conversation-element/host';

vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn(async () => ({
      svg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    })),
  },
}));
const environments: AcceptanceEnvironment[] = [];
const mounted: ReturnType<typeof mount>[] = [];
afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  for (const e of environments.splice(0)) e.dispose();
  document.body.replaceChildren();
  vi.useRealTimers();
});
async function environment() {
  const e = new AcceptanceEnvironment();
  environments.push(e);
  const events: ConversationTransportEvent[] = [];
  await e.connect(
    { conversationId: 'acceptance-main' },
    {
      event: (event) => events.push(event),
      connection: () => {},
      error: () => {},
    },
  );
  return { e, events };
}
function snapshot(events: ConversationTransportEvent[]) {
  const last = events.filter((e) => e.type === 'state-snapshot').at(-1)!;
  if (last.type !== 'state-snapshot') throw new Error('Missing snapshot');
  return last.payload;
}

it('keeps the complete content fixture parseable as Markdown, formula, diagram and image', async () => {
  const { events } = await environment();
  const text = snapshot(events).ledger_records!.find(
    (record) => record.role === 'assistant',
  )!.content;
  const parts = parseConversationContent(text);
  expect(parts.some((part) => part.kind === 'mermaid')).toBe(true);
  expect(
    parts
      .filter((part) => part.kind === 'markdown')
      .map((part) => part.content)
      .join('\n'),
  ).toContain('![验收图片]');
});

it('enables Lit model, history, image and approval controls with the same fixture', async () => {
  const e = new AcceptanceEnvironment();
  environments.push(e);
  const element = new AgentRuntimeConversationElement();
  element.transport = e;
  element.capabilities = e.capabilities;
  element.conversationId = 'acceptance-main';
  element.locale = 'zh-CN';
  element.providerControls = {
    enabled: true,
    controller: e.providerController,
  };
  element.persistence = { enabled: true, controller: e.persistenceController };
  document.body.append(element);
  await element.connect();
  await element.updateComplete;
  await Promise.resolve();
  await element.updateComplete;
  try {
    const rich = Array.from(
      element.shadowRoot!.querySelectorAll<ConversationRichContentElement>(
        'agent-conversation-rich-content',
      ),
    );
    // Components have their own update cycle inside the conversation's shadow root.
    await Promise.all(rich.map((e) => e.updateComplete));
    expect(
      rich.flatMap((e) =>
        Array.from(
          e.shadowRoot!.querySelectorAll('.tool-state'),
          (label) => label.textContent,
        ),
      ),
    ).toEqual(['成功', '失败', '请求执行', '执行中', '等待批准', '结果待确认']);
    expect(
      rich.flatMap((e) =>
        Array.from(e.shadowRoot!.querySelectorAll('.widget')),
      ),
    ).toHaveLength(5);
    expect(
      element.shadowRoot!.querySelector('[part="provider-actions"]'),
    ).not.toBeNull();
    expect(
      element.shadowRoot!.querySelector('[part="persistence-actions"]'),
    ).not.toBeNull();
    expect(
      element.shadowRoot!.querySelector('input.image-input'),
    ).not.toBeNull();
    e.scenario('approval');
    await element.updateComplete;
    expect(
      element.shadowRoot!.querySelectorAll('.permission-tool'),
    ).toHaveLength(2);
    e.expireApprovals();
    await element.updateComplete;
    expect(element.shadowRoot!.querySelector('.permission-shelf')).toBeNull();
  } finally {
    element.remove();
  }
});

it('shows host-resolved thumbnails, opens their original image and falls back on load failure', async () => {
  const element = new ImageReferenceElement();
  const openImage = vi.fn();
  element.imageId = 'asset-one';
  element.label = 'Preview image';
  element.capabilities = {
    resolveImage: async () => ({ source: 'data:image/png;base64,AA==' }),
    openImage,
  };
  document.body.append(element);
  await element.updateComplete;
  await Promise.resolve();
  await element.updateComplete;
  const img = element.shadowRoot!.querySelector('img')!;
  expect(img).not.toBeNull();
  element.shadowRoot!.querySelector('button')!.click();
  expect(openImage).toHaveBeenCalledWith(
    expect.objectContaining({
      metadata: { imageId: 'asset-one' },
      source: img.src,
    }),
  );
  img.dispatchEvent(new Event('error'));
  await element.updateComplete;
  expect(element.shadowRoot!.querySelector('img')).toBeNull();
  element.remove();
  for (const source of [
    'file:///private/image.png',
    'javascript:alert(1)',
    'data:text/html,test',
  ])
    expect(isSafeImageSource(source)).toBe(false);
  expect(isSafeImageSource('data:image/svg+xml,%3Csvg%3E')).toBe(true);
});

it('ignores late image resolution when a reference changes or is removed', async () => {
  const pending = new Map<string, (value: { source: string }) => void>();
  const element = new ImageReferenceElement();
  element.capabilities = {
    resolveImage: ({ imageId }) =>
      new Promise((resolve) => pending.set(imageId, resolve)),
  };
  element.imageId = 'old';
  document.body.append(element);
  await element.updateComplete;
  element.imageId = 'new';
  await element.updateComplete;
  pending.get('new')!({ source: 'https://example.com/new.png' });
  await Promise.resolve();
  await element.updateComplete;
  pending.get('old')!({ source: 'https://example.com/old.png' });
  await Promise.resolve();
  await element.updateComplete;
  expect(element.shadowRoot!.querySelector('img')!.src).toBe(
    'https://example.com/new.png',
  );
  element.imageId = 'removed';
  await element.updateComplete;
  element.remove();
  pending.get('removed')!({ source: 'https://example.com/removed.png' });
  await Promise.resolve();
  await element.updateComplete;
  expect(element.shadowRoot!.querySelector('img')).toBeNull();
});

it('serves every review scenario from a shared local environment', async () => {
  const { e, events } = await environment();
  for (const [id] of scenarios) {
    e.scenario(id);
    expect(snapshot(events).conversation_state).toBe(
      id === 'approval' || id === 'tools'
        ? 'executing'
        : id === 'stream'
          ? 'thinking'
          : 'waiting',
    );
  }
  e.scenario('approval');
  expect(snapshot(events).pending_permissions).toHaveLength(2);
  const permissions = snapshot(events).pending_permissions!;
  await e.resolveToolPermission({
    toolCallId: permissions[0].tool_call_id,
    decision: 'allow',
  });
  expect(snapshot(events).pending_permissions).toHaveLength(1);
  e.expireApprovals();
  expect(snapshot(events).pending_permissions).toHaveLength(0);
  expect(events.some((event) => event.type === 'transport-extension')).toBe(
    true,
  );
});
it('streams, pauses, and completes the plan without contacting a provider', async () => {
  vi.useFakeTimers();
  const { e, events } = await environment();
  e.scenario('stream');
  await vi.advanceTimersByTimeAsync(350);
  expect(snapshot(events).assistant_stream?.content).toBeTruthy();
  await e.pause();
  expect(snapshot(events).conversation_state).toBe('waiting');
  expect(vi.getTimerCount()).toBe(0);
  e.scenario('plan');
  e.advancePlan();
  e.advancePlan();
  expect(snapshot(events).plan?.status).toBe('finished');
  e.compact();
  expect(snapshot(events).conversation_state).toBe('compacting');
  await vi.advanceTimersByTimeAsync(1700);
  expect(snapshot(events).conversation_state).toBe('waiting');
});
it('saves and restores isolated snapshots and keeps models on the selected conversation', async () => {
  const { e, events } = await environment();
  e.scenario('widgets');
  const saved = await e.persistenceController.save({
    runtimeConversationId: 'acceptance-main',
  });
  e.scenario('empty');
  expect(snapshot(events).ledger_records).toHaveLength(0);
  const restored = await e.persistenceController.restore({
    archiveId: saved.archiveId,
  });
  await e.connect(
    { conversationId: restored.runtimeConversationId },
    {
      event: (event) => events.push(event),
      connection: () => {},
      error: () => {},
    },
  );
  expect(snapshot(events).ledger_records?.[0].content).toContain('交互表单');
  await e.providerController.setConversationModel({
    conversationId: restored.runtimeConversationId,
    modelUid: 1003,
  });
  expect(snapshot(events).model_uid).toBe(1003);
});
it('renders the complete Svelte acceptance shell and switches scenarios and appearance', async () => {
  const target = document.createElement('div');
  document.body.append(target);
  const component = mount(App, { target });
  mounted.push(component);
  await tick();
  await Promise.resolve();
  await tick();
  expect(target.textContent).toContain('Svelte 前端验收');
  expect(target.querySelectorAll('[name="scenario"] option')).toHaveLength(
    scenarios.length,
  );
  expect(target.querySelector('.model-name')?.textContent).toBe('chat-demo');
  expect(target.querySelectorAll('details.tool')).toHaveLength(6);
  expect(
    Array.from(target.querySelectorAll('.tool-state'), (e) => e.textContent),
  ).toEqual(['成功', '失败', '请求执行', '执行中', '等待批准', '结果待确认']);
  expect(target.querySelectorAll('.widget')).toHaveLength(5);
  expect(
    target.querySelector('.widgets[data-inactive="false"]'),
  ).not.toBeNull();
  expect(
    target.querySelector('.orbit-svelte-image-reference img'),
  ).not.toBeNull();
  const scenario =
    target.querySelector<HTMLSelectElement>('[name="scenario"]')!;
  scenario.value = 'approval';
  scenario.dispatchEvent(new Event('change', { bubbles: true }));
  await tick();
  expect(target.querySelectorAll('.permission-tool')).toHaveLength(2);
  const theme = target.querySelector<HTMLSelectElement>('[name="theme"]')!;
  theme.value = 'paper';
  theme.dispatchEvent(new Event('change', { bubbles: true }));
  await tick();
  expect(target.querySelector('[data-theme="paper"]')).not.toBeNull();
  target.querySelector<HTMLButtonElement>('[data-action="expire"]')!.click();
  await tick();
  expect(target.querySelector('.permission-shelf')).toBeNull();
});
