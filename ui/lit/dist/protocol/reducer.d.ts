import type { ConversationAction, ConversationState, PendingUserMessage, ExecutionPlan } from './types.js';
export declare function createConversationState(conversationId?: string | null): ConversationState;
export declare function conversationReducer(state: ConversationState, action: ConversationAction): ConversationState;
export declare function normalizePlan(value: unknown): ExecutionPlan | undefined;
export declare function createPendingUserMessage(id: string, content: string, createdAt?: string, parts?: PendingUserMessage['parts']): PendingUserMessage;
//# sourceMappingURL=reducer.d.ts.map