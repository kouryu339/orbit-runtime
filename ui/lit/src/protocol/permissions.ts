import type { PendingToolPermission } from './types.js';

export type PermissionTargetSummary = {
  label: string;
  value: string;
};

const SAFE_TARGET_KEYS = [
  'path',
  'file',
  'directory',
  'url',
  'selector',
  'page_id',
  'workflow_id',
  'node_id',
  'name',
  'query',
  'operation',
] as const;

const SENSITIVE_KEY = /(token|secret|password|api[_-]?key|authorization|cookie|body|content)/i;

/**
 * Build a short, non-JSON approval summary. Runtime keeps the full arguments
 * for execution and audit; the regular UI only exposes safe target fields.
 */
export function summarizePermissionTargets(
  permission: PendingToolPermission,
): PermissionTargetSummary[] {
  const argumentsValue = permission.arguments ?? {};
  const summaries: PermissionTargetSummary[] = [];

  for (const key of SAFE_TARGET_KEYS) {
    if (SENSITIVE_KEY.test(key) || !(key in argumentsValue)) continue;
    const value = readableScalar(argumentsValue[key]);
    if (!value) continue;
    summaries.push({
      label: key.replaceAll('_', ' '),
      value: value.length > 180 ? `${value.slice(0, 177)}…` : value,
    });
    if (summaries.length === 4) break;
  }
  return summaries;
}

function readableScalar(value: unknown): string {
  if (typeof value === 'string') return value.trim();
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) {
    const values = value
      .filter((item) => ['string', 'number', 'boolean'].includes(typeof item))
      .slice(0, 4)
      .map(String);
    return values.join(', ');
  }
  return '';
}
