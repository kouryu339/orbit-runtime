import React from 'react';

/**
 * Template pattern: matches `→{{name}}` (output) and `{{name}}` (input).
 * The arrow prefix `→` distinguishes output parameters from input parameters.
 */
const TPL_RE = /(→\{\{(\w+)\}\}|\{\{(\w+)\}\})/g;

/* ─── styles ─────────────────────────────────────────── */
const baseStyle: React.CSSProperties = {
  fontWeight: 600,
  borderRadius: 3,
  padding: '0 3px',
  display: 'inline',
};

const inputStyle: React.CSSProperties = {
  ...baseStyle,
  color: '#3b82f6',
  background: 'rgba(59,130,246,0.1)',
};

const outputStyle: React.CSSProperties = {
  ...baseStyle,
  color: '#22c55e',
  background: 'rgba(34,197,94,0.1)',
};

/**
 * Render a description template string into React nodes with highlighted parameters.
 *
 * - `{{name}}`  → blue highlighted input param
 * - `→{{name}}` → green highlighted output param
 * - Plain text is kept as-is
 */
export function renderDescription(template: string): React.ReactNode[] {
  const parts: React.ReactNode[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  TPL_RE.lastIndex = 0;
  while ((match = TPL_RE.exec(template)) !== null) {
    // text before this match
    if (match.index > lastIndex) {
      parts.push(template.slice(lastIndex, match.index));
    }

    const isOutput = match[1].startsWith('→');
    const paramName = isOutput ? match[2] : match[3];

    parts.push(
      <span key={match.index} style={isOutput ? outputStyle : inputStyle}>
        {isOutput ? `→${paramName}` : paramName}
      </span>,
    );

    lastIndex = match.index + match[0].length;
  }

  // trailing text
  if (lastIndex < template.length) {
    parts.push(template.slice(lastIndex));
  }

  return parts;
}

/**
 * Format a description template into plain text (for title/tooltip attributes).
 *
 * - `{{name}}`  → `[name]`
 * - `→{{name}}` → `→[name]`
 */
export function formatDescription(template: string): string {
  return template
    .replace(/→\{\{(\w+)\}\}/g, '→[$1]')
    .replace(/\{\{(\w+)\}\}/g, '[$1]');
}
