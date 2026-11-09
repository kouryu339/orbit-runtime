import { fileURLToPath } from 'node:url';

// Resolve the shared modules from this checkout, even when npm's file link
// still points at a different worktree.
export const sharedConversationAlias = Object.fromEntries(
  ['host', 'protocol', 'content', 'transport'].map((name) => [
    `@agent-runtime/conversation-element/${name}`,
    fileURLToPath(new URL(`../lit/src/${name}/index.ts`, import.meta.url)),
  ]),
);
