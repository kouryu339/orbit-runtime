// Color constants and pin type → color mapping

export const NODE_COLORS = {
  start: '#22c55e',
  end: '#ef4444',
  action: '#3b82f6',
  decision: '#f59e0b',
  loop: '#8b5cf6',
  break: '#ef4444',
  variable: '#14b8a6',
  sub_process: '#6366f1',
} as const;

export const DATA_TYPE_COLORS: Record<string, string> = {
  String: 'rgba(42, 105, 190, 0.96)',
  Path: 'rgba(42, 105, 190, 0.96)',
  Date: 'rgba(42, 105, 190, 0.96)',
  Time: 'rgba(42, 105, 190, 0.96)',
  f64: 'rgba(30, 145, 92, 0.96)',
  i64: 'rgba(30, 145, 92, 0.96)',
  Number: 'rgba(30, 145, 92, 0.96)',
  bool: 'rgba(201, 55, 74, 0.96)',
  Boolean: 'rgba(201, 55, 74, 0.96)',
  Object: 'rgba(105, 113, 125, 0.92)',
  Any: 'rgba(105, 113, 125, 0.92)',
};

export const EXEC_COLOR = 'rgba(45, 35, 72, 0.96)';

export const CATEGORY_COLORS: Record<string, string> = {
  Flow: '#3b82f6',
  IO: '#22c55e',
  Data: '#f59e0b',
  AI: '#8b5cf6',
  Network: '#06b6d4',
  Default: '#6b7280',
};

export function normalizeDataType(dataType: string): string {
  const trimmed = dataType.trim();
  if (!trimmed) return 'Any';
  const lower = trimmed.toLowerCase();
  if (lower === 'str' || lower === 'string') return 'String';
  if (lower === 'integer' || lower === 'int' || lower === 'i64') return 'i64';
  if (lower === 'float' || lower === 'f64' || lower === 'number') return lower === 'number' ? 'Number' : 'f64';
  if (lower === 'bool' || lower === 'boolean') return 'bool';
  if (lower === 'any') return 'Any';
  if (lower === 'object') return 'Object';
  return trimmed;
}

export function getArrayElementType(dataType: string): string | null {
  const trimmed = dataType.trim();
  const generic = /^(?:array|vec)\s*<\s*(.+?)\s*>$/i.exec(trimmed);
  if (generic) return normalizeDataType(generic[1]);
  const bracketGeneric = /^(?:array|vec)\s*\[\s*(.+?)\s*\]$/i.exec(trimmed);
  if (bracketGeneric) return normalizeDataType(bracketGeneric[1]);
  const suffix = /^(.+?)\s*\[\s*\]$/.exec(trimmed);
  return suffix ? normalizeDataType(suffix[1]) : null;
}

export function isArrayDataType(dataType: string): boolean {
  return getArrayElementType(dataType) !== null || normalizeDataType(dataType) === 'Array';
}

export function getDataTypeColor(dataType: string): string {
  const normalized = normalizeDataType(dataType);
  const elementType = getArrayElementType(normalized);
  if (elementType) {
    return getDataTypeColor(elementType);
  }
  return DATA_TYPE_COLORS[normalized] ?? DATA_TYPE_COLORS.Any;
}

export function getPinAccentColor(dataType: string): string {
  return getDataTypeColor(dataType);
}

export function getCategoryColor(category: string): string {
  return CATEGORY_COLORS[category] ?? CATEGORY_COLORS['Default'];
}
