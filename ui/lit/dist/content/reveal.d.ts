import { parseConversationContent } from './parser.js';
export declare function sourceLength(part: ReturnType<typeof parseConversationContent>[number]): number;
export declare function revealBaseline(content: string): number;
export declare function nextRevealLength(content: string, current: number, step: number): number;
export declare function isSafeLink(url: string): boolean;
//# sourceMappingURL=reveal.d.ts.map