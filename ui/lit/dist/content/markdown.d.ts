import type { ConversationHostCapabilities } from '../host/types.js';
export declare function renderSafeMarkdown(source: string): string;
/** Route link activation to the host without navigating the conversation view. */
export declare function handleMarkdownLink(event: MouseEvent, capabilities: ConversationHostCapabilities, onError: (error: unknown) => void): boolean;
//# sourceMappingURL=markdown.d.ts.map