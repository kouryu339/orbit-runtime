import type { ConversationAction, ConversationState, PendingUserMessage } from './types.js';
export declare function createConversationState(conversationId?: string | null): ConversationState;
export declare function conversationReducer(state: ConversationState, action: ConversationAction): ConversationState;
export declare function createPendingUserMessage(id: string, content: string, createdAt?: string): PendingUserMessage;
//# sourceMappingURL=reducer.d.ts.map