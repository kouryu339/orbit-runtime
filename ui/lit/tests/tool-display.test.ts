import { expect, it } from 'vitest';
import { toolCallFromRecord } from '../src/protocol/records.js';
import { toolDisplaySummary } from '../src/content/parser.js';

it('never projects raw output, commands or error text into tool display fields', () => {
  const raw = '{"secret":"RAW_PRIVATE_RESULT","stack":"internal failure"}';
  const call = toolCallFromRecord({
    role: 'tool',
    content: raw,
    metadata: {
      subtype: 'tool_call_failed',
      tool_command: 'Write --token PRIVATE_COMMAND',
      extra: { call_id: 'one' },
    },
  })!;
  expect(call.title).toBe('Tool call');
  expect(call.detail).toBe('');
  expect(call.command).toBeUndefined();
  expect(toolDisplaySummary(call, 'zh-CN')).toBe(
    '工具执行失败，请检查后重试。',
  );
  expect(JSON.stringify(call)).not.toContain('RAW_PRIVATE_RESULT');
  expect(JSON.stringify(call)).not.toContain('PRIVATE_COMMAND');
});
it('uses only an explicit user summary and rejects serialized payloads as summaries', () => {
  const call = toolCallFromRecord({
    role: 'tool',
    content: 'RAW_RESULT',
    metadata: {
      subtype: 'tool_call_finished',
      tool_name: 'Read',
      display_content: 'RAW_DISPLAY_RESULT',
      extra: { call_id: 'one', user_summary: '已处理 12 条记录。' },
    },
  })!;
  expect(toolDisplaySummary(call, 'zh-CN')).toBe('已处理 12 条记录。');
  for (const userSummary of [
    '{"secret":"HIDDEN"}',
    '["HIDDEN"]',
    '```json\nHIDDEN\n```',
    'line one\nHIDDEN',
  ])
    expect(toolDisplaySummary({ ...call, userSummary }, 'en-US')).toBe(
      'The tool completed.',
    );
});
