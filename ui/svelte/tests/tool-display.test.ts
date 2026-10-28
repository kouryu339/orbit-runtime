import { afterEach, describe, expect, it } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import RichContent from '../src/lib/RichContent.svelte';
import { ConversationRichContentElement } from '../../lit/src/components/rich-content.js';
import type { ToolCallView } from '@agent-runtime/conversation-element/protocol';

const cleanup: Array<() => void | Promise<void>> = [];
afterEach(async () => {
  for (const dispose of cleanup.splice(0)) await dispose();
  document.body.replaceChildren();
});
async function render(kind: string, calls: ToolCallView[]) {
  const content = calls
    .map((call) => `[tool:status | call_id="${call.id}"]`)
    .join('\n');
  const target = document.createElement('div');
  document.body.append(target);
  if (kind === 'lit') {
    const element = new ConversationRichContentElement();
    element.content = content;
    element.toolCalls = calls;
    element.locale = 'zh-CN';
    target.append(element);
    await element.updateComplete;
    cleanup.push(() => element.remove());
    return element.shadowRoot!;
  }
  const instance = mount(RichContent, {
    target,
    props: { content, toolCalls: calls, locale: 'zh-CN' },
  });
  cleanup.push(() => unmount(instance));
  await tick();
  return target;
}
describe.each(['lit', 'svelte'])('%s user-facing tool details', (kind) => {
  it('renders readable state summaries without mounting raw JSON, commands or raw errors', async () => {
    const statuses: ToolCallView['status'][] = [
      'placeholder',
      'running',
      'waiting_permission',
      'finished',
      'failed',
      'uncertain',
    ];
    const root = await render(
      kind,
      statuses.map((status, index) => ({
        id: String(index),
        title: '处理记录',
        toolName: 'Process',
        status,
        detail: '{"private":"RAW_RESULT"}',
        command: 'Process --token RAW_COMMAND',
      })),
    );
    for (const details of root.querySelectorAll('details')) details.open = true;
    expect(root.querySelectorAll('.tool-detail')).toHaveLength(6);
    expect(root.textContent).not.toContain('RAW_RESULT');
    expect(root.textContent).not.toContain('RAW_COMMAND');
    expect(root.textContent).toContain('等待你的批准后执行。');
    expect(root.textContent).toContain('工具已执行完成。');
    expect(root.textContent).toContain('工具执行失败');
    expect(root.querySelector('pre.tool-detail')).toBeNull();
    expect(root.querySelector('.tool-command')).toBeNull();
  });
  it('uses host-provided user prose while ignoring legacy details and serialized summaries', async () => {
    const root = await render(kind, [
      {
        id: 'safe',
        title: '读取数据',
        status: 'finished',
        toolName: 'Read',
        detail: 'PRIVATE_DETAILS',
        userSummary: '已读取 12 条记录。',
      },
      {
        id: 'unsafe',
        title: '读取数据',
        status: 'finished',
        toolName: 'Read',
        detail: 'PRIVATE_DETAILS',
        userSummary: '{"internal":"PRIVATE_SUMMARY"}',
      },
    ]);
    expect(root.textContent).toContain('已读取 12 条记录。');
    expect(root.textContent).toContain('工具已执行完成。');
    expect(root.textContent).not.toContain('PRIVATE_');
  });
});
