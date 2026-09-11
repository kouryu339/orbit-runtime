const TOOL_LINE = /^\[tool:status\s*\|\s*call_id="([^"]+)"\]$/;
const WIDGET_LINE = /^\[(input:path|input:text|input:date|input:time|select:single|select:multi|confirm)\s*(?:\|\s*(.*))?\]$/;
export function parseConversationContent(content) {
    const parts = [];
    const markdown = [];
    const widgets = [];
    let fence = null;
    const flushMarkdown = () => {
        const value = markdown.join('\n').trim();
        if (value)
            parts.push({ kind: 'markdown', content: value });
        markdown.length = 0;
    };
    const flushWidgets = () => {
        if (widgets.length)
            parts.push({ kind: 'widgets', widgets: [...widgets] });
        widgets.length = 0;
    };
    for (const rawLine of content.split(/\r?\n/)) {
        const trimmed = rawLine.trim();
        if (fence) {
            if (trimmed.startsWith('```')) {
                if (fence.language === 'mermaid') {
                    flushMarkdown();
                    flushWidgets();
                    parts.push({ kind: 'mermaid', source: fence.lines.join('\n') });
                }
                else {
                    markdown.push(`\`\`\`${fence.language}`, ...fence.lines, '```');
                }
                fence = null;
            }
            else {
                fence.lines.push(rawLine);
            }
            continue;
        }
        if (trimmed.startsWith('```')) {
            flushWidgets();
            fence = {
                language: trimmed.slice(3).trim().toLowerCase(),
                lines: [],
            };
            continue;
        }
        const tool = trimmed.match(TOOL_LINE);
        if (tool) {
            flushMarkdown();
            flushWidgets();
            parts.push({ kind: 'tool', callId: tool[1] });
            continue;
        }
        const widget = parseWidget(trimmed);
        if (widget) {
            flushMarkdown();
            widgets.push(widget);
            continue;
        }
        flushWidgets();
        markdown.push(rawLine);
    }
    if (fence)
        markdown.push(`\`\`\`${fence.language}`, ...fence.lines);
    flushMarkdown();
    flushWidgets();
    return parts;
}
export function parseWidget(line) {
    const match = line.match(WIDGET_LINE);
    if (!match)
        return null;
    const properties = new Map();
    for (const segment of (match[2] ?? '').split('|')) {
        const property = segment.trim().match(/^([a-zA-Z_]+)="([^"]*)"$/);
        if (property)
            properties.set(property[1], property[2]);
    }
    const label = properties.get('label');
    if (!label)
        return null;
    return {
        kind: match[1],
        label,
        options: (properties.get('options') ?? '')
            .split(',')
            .map((item) => item.trim())
            .filter(Boolean),
        accept: properties.get('accept'),
        raw: line,
    };
}
export function toolCallForId(calls, callId) {
    return (calls.find((call) => call.id === callId) ?? {
        id: callId,
        title: 'Preparing tool call',
        status: 'placeholder',
        detail: '',
        toolName: '',
    });
}
//# sourceMappingURL=parser.js.map