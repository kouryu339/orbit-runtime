import type { ConversationPresentationItem } from '../host/types.js';

export function visiblePresentationItems(
  items: readonly ConversationPresentationItem[],
  completedKeys: ReadonlySet<string>,
): ConversationPresentationItem[] {
  let pendingProgressiveIncluded = false;
  return items.filter((item) => {
    const key = `presentation:${item.id}`;
    if (item.reveal !== 'progressive' || completedKeys.has(key)) return true;
    if (pendingProgressiveIncluded) return false;
    pendingProgressiveIncluded = true;
    return true;
  });
}
