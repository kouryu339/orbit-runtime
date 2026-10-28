import type { ToolCallView } from '../protocol/types.js';

export type WidgetKind =
  | 'input:path'
  | 'input:text'
  | 'select:single'
  | 'select:multi'
  | 'confirm';

export type WidgetDefinition = {
  kind: WidgetKind;
  label: string;
  options: string[];
  accept?: string;
  raw: string;
};

export type ConversationContentPart =
  | { kind: 'markdown'; content: string }
  | { kind: 'mermaid'; source: string }
  | { kind: 'tool'; callId: string }
  | { kind: 'widgets'; widgets: WidgetDefinition[] };

const TOOL_LINE = /^\[tool:status\s*\|\s*call_id="([^"]+)"\]$/;
const WIDGET_LINE =
  /^\[(input:path|input:text|select:single|select:multi|confirm)\s*(?:\|\s*(.*))?\]$/;

export function parseConversationContent(
  content: string,
): ConversationContentPart[] {
  const parts: ConversationContentPart[] = [];
  const markdown: string[] = [];
  const widgets: WidgetDefinition[] = [];
  let fence: { language: string; lines: string[] } | null = null;

  const flushMarkdown = () => {
    const value = markdown.join('\n').trim();
    if (value) parts.push({ kind: 'markdown', content: value });
    markdown.length = 0;
  };
  const flushWidgets = () => {
    if (widgets.length) parts.push({ kind: 'widgets', widgets: [...widgets] });
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
        } else {
          markdown.push(`\`\`\`${fence.language}`, ...fence.lines, '```');
        }
        fence = null;
      } else {
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

  if (fence) markdown.push(`\`\`\`${fence.language}`, ...fence.lines);
  flushMarkdown();
  flushWidgets();
  return parts;
}

export function parseWidget(line: string): WidgetDefinition | null {
  const match = line.match(WIDGET_LINE);
  if (!match) return null;
  const properties = new Map<string, string>();
  for (const segment of (match[2] ?? '').split('|')) {
    const property = segment.trim().match(/^([a-zA-Z_]+)="([^"]*)"$/);
    if (property) properties.set(property[1], property[2]);
  }
  const label = properties.get('label');
  if (!label) return null;
  return {
    kind: match[1] as WidgetKind,
    label,
    options: (properties.get('options') ?? '')
      .split(',')
      .map((item) => item.trim())
      .filter(Boolean),
    accept: properties.get('accept'),
    raw: line,
  };
}

export function toolCallForId(
  calls: ToolCallView[],
  callId: string,
): ToolCallView {
  return (
    calls.find((call) => call.id === callId) ?? {
      id: callId,
      title: 'Preparing tool call',
      status: 'placeholder',
      detail: '',
      toolName: '',
    }
  );
}

export function toolStatusLabel(
  status: ToolCallView['status'],
  locale: string,
): string {
  const labels = {
    placeholder: ['请求执行', 'Requested'],
    running: ['执行中', 'Running'],
    waiting_permission: ['等待批准', 'Waiting for approval'],
    finished: ['成功', 'Succeeded'],
    failed: ['失败', 'Failed'],
    uncertain: ['结果待确认', 'Result unknown'],
  };
  return labels[status][locale === 'zh-CN' ? 0 : 1];
}

/** No raw-result, raw-error or command fallback, including for legacy callers. */
export function toolDisplaySummary(call: ToolCallView, locale: string): string {
  const summary = call.userSummary?.trim();
  // This field is for short prose, never serialized payloads or code blocks.
  if (
    summary &&
    summary.length <= 280 &&
    !/[\r\n]/.test(summary) &&
    !/^[\[{"`]/.test(summary)
  )
    return summary;
  const messages: Record<ToolCallView['status'], [string, string]> = {
    placeholder: ['正在准备工具调用。', 'Preparing the tool call.'],
    running: ['工具正在执行，请稍候。', 'The tool is running.'],
    waiting_permission: ['等待你的批准后执行。', 'Waiting for your approval.'],
    finished: ['工具已执行完成。', 'The tool completed.'],
    failed: [
      '工具执行失败，请检查后重试。',
      'The tool failed. Check the task before retrying.',
    ],
    uncertain: [
      '执行结果尚未确认，请先核实任务状态。',
      'The result is unconfirmed. Check the task state first.',
    ],
  };
  return messages[call.status][locale.startsWith('zh') ? 0 : 1];
}
