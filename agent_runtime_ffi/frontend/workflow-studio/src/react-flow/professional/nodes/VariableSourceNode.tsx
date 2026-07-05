import React, { useEffect, useRef, useState } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import type { EditorEvent } from '../../types';
import { getDataTypeColor, getPinAccentColor, isArrayDataType } from '../../theme';

interface VariableSourceData {
  name: string;
  dataType: string;
  defaultValue?: unknown;
  readonly?: boolean;
  connected?: boolean;
  onEvent?: (event: EditorEvent) => void;
}

function formatValue(value: unknown): string {
  if (value === undefined || value === null) return '';
  return typeof value === 'string' ? value : JSON.stringify(value);
}

function parseValue(raw: string, dataType: string, fallback: unknown): unknown {
  const text = raw.trim();
  const normalized = dataType.trim();
  if (text === '') return null;
  if (normalized === 'String' || normalized === 'Path' || normalized === 'Date' || normalized === 'Time') return raw;
  if (normalized === 'bool' || normalized === 'Boolean') return text === 'true' || text === '1';
  if (normalized === 'i64' || normalized === 'int' || normalized === 'integer') {
    const value = Number.parseInt(text, 10);
    return Number.isNaN(value) ? fallback : value;
  }
  if (normalized === 'f64' || normalized === 'float' || normalized === 'Number') {
    const value = Number.parseFloat(text);
    return Number.isNaN(value) ? fallback : value;
  }
  if (isArrayDataType(normalized) || normalized === 'Object' || normalized === 'Any') {
    try {
      return JSON.parse(text);
    } catch {
      return isArrayDataType(normalized) || normalized === 'Object' ? fallback : raw;
    }
  }
  return raw;
}

export function VariableSourceNode({ data }: NodeProps) {
  const d = data as unknown as VariableSourceData;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(() => formatValue(d.defaultValue));
  const inputRef = useRef<HTMLInputElement>(null);
  const typeColor = getDataTypeColor(d.dataType || 'Any');
  const accent = getPinAccentColor(d.dataType || 'Any');

  useEffect(() => setDraft(formatValue(d.defaultValue)), [d.defaultValue]);
  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  const commit = () => {
    setEditing(false);
    d.onEvent?.({
      type: 'varupdate',
      payload: {
        name: d.name,
        defaultValue: parseValue(draft, d.dataType || 'Any', d.defaultValue),
      },
    });
  };

  return (
    <div
      style={{
        width: 150,
        background: '#f0fdfa',
        border: `1px solid ${accent}66`,
        borderLeft: `3px solid ${accent}`,
        borderRadius: 6,
        boxShadow: '0 4px 14px rgba(73,104,97,0.12)',
        color: '#134e4a',
        fontFamily: '"Segoe UI", sans-serif',
        padding: '7px 10px',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
        <span
          title={`$${d.name}: ${d.dataType}`}
          style={{
            color: accent,
            fontSize: 12,
            fontWeight: 700,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          ${d.name}
        </span>
        <span style={{ color: typeColor, fontSize: 10, fontFamily: '"Cascadia Code", monospace', flexShrink: 0 }}>
          {d.dataType || 'Any'}
        </span>
      </div>

      {editing && !d.readonly ? (
        <input
          ref={inputRef}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onClick={(event) => event.stopPropagation()}
          onKeyDown={(event) => {
            if (event.key === 'Enter') commit();
            if (event.key === 'Escape') {
              setDraft(formatValue(d.defaultValue));
              setEditing(false);
            }
            event.stopPropagation();
          }}
          style={{
            width: '100%',
            marginTop: 6,
            boxSizing: 'border-box',
            border: `1px solid ${accent}66`,
            borderRadius: 3,
            background: '#fffefa',
            color: '#304944',
            fontSize: 11,
            fontFamily: '"Cascadia Code", monospace',
            padding: '2px 4px',
            outline: 'none',
          }}
        />
      ) : (
        <div
          title={d.readonly ? formatValue(d.defaultValue) : 'Edit default value'}
          onDoubleClick={(event) => {
            if (d.readonly) return;
            event.stopPropagation();
            setEditing(true);
          }}
          style={{
            marginTop: 5,
            color: '#718781',
            fontSize: 11,
            fontFamily: '"Cascadia Code", monospace',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            cursor: d.readonly ? 'default' : 'text',
            minHeight: 15,
          }}
        >
          {formatValue(d.defaultValue) || 'null'}
        </div>
      )}

      <Handle
        type="source"
        id="Value"
        position={Position.Right}
        className="blueprint-pin-handle"
        style={{
          width: isArrayDataType(d.dataType || 'Any') ? 20 : 16,
          height: 16,
          right: 1,
          top: '50%',
          transform: 'translateY(-50%)',
        }}
      >
        <span className="blueprint-data-pin" style={{ color: accent }}>
          {isArrayDataType(d.dataType || 'Any') && <span className="blueprint-array-bracket">[</span>}
          <span
            className="blueprint-data-pin-dot"
            style={{
              width: isArrayDataType(d.dataType || 'Any') ? 4 : undefined,
              height: isArrayDataType(d.dataType || 'Any') ? 4 : undefined,
              background: d.connected ? accent : '#fffefa',
              borderColor: accent,
            }}
          />
          {isArrayDataType(d.dataType || 'Any') && <span className="blueprint-array-bracket">]</span>}
        </span>
      </Handle>
    </div>
  );
}
