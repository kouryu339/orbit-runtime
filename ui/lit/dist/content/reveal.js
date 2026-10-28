import { parseConversationContent } from './parser.js';
export function sourceLength(part) {
    if (part.kind === 'mermaid')
        return part.source.length;
    if (part.kind === 'tool')
        return part.callId.length;
    if (part.kind === 'widgets') {
        return part.widgets.reduce((total, widget) => total + widget.raw.length, 0);
    }
    return part.content.length;
}
export function revealBaseline(content) {
    const parts = parseConversationContent(content);
    let lastMarkdownIndex = -1;
    for (let index = parts.length - 1; index >= 0; index -= 1) {
        if (parts[index].kind === 'markdown') {
            lastMarkdownIndex = index;
            break;
        }
    }
    if (lastMarkdownIndex < 0)
        return content.length;
    return parts
        .slice(0, lastMarkdownIndex)
        .reduce((total, part) => total + sourceLength(part), 0);
}
export function nextRevealLength(content, current, step) {
    let next = Math.min(content.length, current + step);
    const atomicInlinePattern = /!?\[[^\]]*\]\([^)]+\)/g;
    let match;
    while ((match = atomicInlinePattern.exec(content)) !== null) {
        const end = match.index + match[0].length;
        if (next > match.index && next < end) {
            next = end;
            break;
        }
    }
    return next;
}
export function isSafeLink(url) {
    try {
        return ['http:', 'https:', 'mailto:'].includes(new URL(url).protocol);
    }
    catch {
        return false;
    }
}
//# sourceMappingURL=reveal.js.map