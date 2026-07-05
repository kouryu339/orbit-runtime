import { describe, expect, it } from 'vitest';
import { parseConversationContent } from '../src/content/parser.js';

describe('parseConversationContent', () => {
  it('preserves markdown, tool, widget, and mermaid ordering', () => {
    const parts = parseConversationContent([
      'I will inspect the graph.',
      '[tool:status | call_id="call-1"]',
      '```mermaid',
      'graph TD',
      'A --> B',
      '```',
      'Choose a format:',
      '[select:single | label="Format" | options="SVG,PNG"]',
    ].join('\n'));

    expect(parts).toEqual([
      { kind: 'markdown', content: 'I will inspect the graph.' },
      { kind: 'tool', callId: 'call-1' },
      { kind: 'mermaid', source: 'graph TD\nA --> B' },
      { kind: 'markdown', content: 'Choose a format:' },
      {
        kind: 'widgets',
        widgets: [
          {
            kind: 'select:single',
            label: 'Format',
            options: ['SVG', 'PNG'],
            accept: undefined,
            raw: '[select:single | label="Format" | options="SVG,PNG"]',
          },
        ],
      },
    ]);
  });

  it('does not parse protocol-looking lines inside code fences', () => {
    const parts = parseConversationContent([
      '```text',
      '[tool:status | call_id="example"]',
      '[input:text | label="Example"]',
      '```',
    ].join('\n'));

    expect(parts).toEqual([
      {
        kind: 'markdown',
        content: [
          '```text',
          '[tool:status | call_id="example"]',
          '[input:text | label="Example"]',
          '```',
        ].join('\n'),
      },
    ]);
  });

  it('keeps latex syntax in markdown for the math renderer', () => {
    const parts = parseConversationContent(
      'Inline $E = mc^2$.\n\n$$\\int_0^1 x^2 dx$$',
    );
    expect(parts).toEqual([
      {
        kind: 'markdown',
        content: 'Inline $E = mc^2$.\n\n$$\\int_0^1 x^2 dx$$',
      },
    ]);
  });
});
