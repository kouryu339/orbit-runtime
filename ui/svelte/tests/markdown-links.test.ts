import { afterEach, describe, expect, it, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import type { ConversationHostCapabilities } from '@agent-runtime/conversation-element/host';
import { ConversationRichContentElement } from '../../lit/src/components/rich-content.js';
import RichContent from '../src/lib/RichContent.svelte';

const disposals: Array<() => void | Promise<void>> = [];

afterEach(async () => {
  for (const dispose of disposals.splice(0)) await dispose();
  document.body.replaceChildren();
});

async function render(kind: string, capabilities: ConversationHostCapabilities = {}) {
  const content = '[**Report**](https://example.invalid/report)\n\n[Phone](tel:12345)';
  const host = document.createElement('div');
  document.body.append(host);
  if (kind === 'Lit') {
    const element = new ConversationRichContentElement();
    element.content = content;
    element.capabilities = capabilities;
    host.append(element);
    await element.updateComplete;
    return { host, root: element.shadowRoot! };
  }
  const instance = mount(RichContent, { target: host, props: { content, capabilities } });
  disposals.push(() => unmount(instance));
  await tick();
  return { host, root: host };
}

function activate(target: Element, type = 'click', options?: MouseEventInit) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, composed: true, ...options });
  target.dispatchEvent(event);
  return event;
}

describe.each(['Lit', 'Svelte'])('%s Markdown link routing', (kind) => {
  it('cancels normal activation and offers the link to the embedding host', async () => {
    const { host, root } = await render(kind);
    const listener = vi.fn();
    host.addEventListener('agent-conversation-link-open', listener);
    expect(activate(root.querySelector('a strong')!).defaultPrevented).toBe(true);
    expect(listener).toHaveBeenCalledOnce();
    expect(listener.mock.calls[0][0].detail).toEqual({
      url: 'https://example.invalid/report',
      source: 'markdown',
    });
    host.removeEventListener('agent-conversation-link-open', listener);
    expect(activate(root.querySelector('a')!).defaultPrevented).toBe(true);
  });

  it('routes modified and middle clicks once, and blocks unapproved URL schemes', async () => {
    const openLink = vi.fn();
    const { host, root } = await render(kind, { openLink });
    const listener = vi.fn();
    host.addEventListener('agent-conversation-link-open', listener);
    const [report, phone] = root.querySelectorAll('a');
    expect(activate(report, 'click', { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(activate(report, 'auxclick', { button: 1 }).defaultPrevented).toBe(true);
    expect(activate(phone).defaultPrevented).toBe(true);
    expect(openLink).toHaveBeenCalledTimes(2);
    expect(openLink).toHaveBeenLastCalledWith({
      url: 'https://example.invalid/report',
      source: 'markdown',
    });
    expect(listener).not.toHaveBeenCalled();
  });
});
