import { describe, expect, it } from 'vitest';
import {
  PRESENTATION_CONTRACT,
  type ConversationPresentationItem,
} from '../src/host/types.js';
import { visiblePresentationItems } from '../src/content/presentation.js';

function preset(
  id: string,
  reveal: ConversationPresentationItem['reveal'] = 'progressive',
): ConversationPresentationItem {
  return {
    contract: PRESENTATION_CONTRACT,
    id,
    kind: 'assistant-markdown',
    anchor: { type: 'tail' },
    content: id,
    reveal,
  };
}

describe('visiblePresentationItems', () => {
  it('reveals progressive presets one at a time', () => {
    const items = [preset('one'), preset('two'), preset('notice', 'none')];
    expect(
      visiblePresentationItems(items, new Set()).map((item) => item.id),
    ).toEqual(['one', 'notice']);
    expect(
      visiblePresentationItems(
        items,
        new Set(['presentation:one']),
      ).map((item) => item.id),
    ).toEqual(['one', 'two', 'notice']);
  });
});
