import React, { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import type { NodePin, EditorEvent } from '../../types';
import {
  getCategoryColor,
  getDataTypeColor,
  getPinAccentColor,
  isArrayDataType,
  EXEC_COLOR,
} from '../../theme';
import { renderDescription } from '../../utils/descriptionTemplate';

/* ─── constants ──────────────────────────────────────── */
const HEADER_HEIGHT  = 32;
const PIN_ROW_H      = 26;
const CHAR_W         = 6.5;   // approx px per char at font-size 11
const PIN_OVERHEAD   = 26;    // icon(12) + gap(5) + padding(4) + handle(5)
const NODE_MIN_W     = 180;
const NODE_MAX_W     = 360;

/* ─── width estimation ───────────────────────────────── */
function estimateNodeWidth(inputPins: NodePin[], outputPins: NodePin[]): number {
  const maxInLen  = inputPins.reduce((m, p) => Math.max(m, p.name.length), 0);
  const maxOutLen = outputPins.reduce((m, p) => Math.max(m, p.name.length), 0);
  const w = (maxInLen + maxOutLen) * CHAR_W + PIN_OVERHEAD * 2 + 16;
  return Math.min(NODE_MAX_W, Math.max(NODE_MIN_W, Math.round(w)));
}

/* ─── helpers ────────────────────────────────────────── */
function isExecPin(kind: string) {
  return kind === 'ExecInput' || kind === 'ExecOutput';
}

/* ─── editable default value widget (UE style) ───────── */

interface DefaultValueWidgetProps {
  pin: NodePin;
  nodeId: string;
  onEvent?: (event: EditorEvent) => void;
  readonly?: boolean;
  suggestions?: string[];
  onPreview?: (value: string) => void;
}

function DefaultValueWidget({ pin, nodeId, onEvent, readonly, suggestions, onPreview }: DefaultValueWidgetProps) {
  const [editing, setEditing] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const val = pin.default_value;
  const color = getDataTypeColor(pin.data_type);
  const dt = pin.data_type.toLowerCase();

  // Focus input when editing starts
  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  const emitChange = useCallback((newValue: unknown) => {
    onEvent?.({
      type: 'pindefaultchange',
      payload: { nodeId, pinName: pin.name, value: newValue },
    });
  }, [onEvent, nodeId, pin.name]);

  // ── Bool: inline checkbox (always editable, UE style) ──
  if (dt === 'bool' || dt === 'boolean') {
    return (
      <div
        onClick={(e) => {
          if (readonly) return;
          e.stopPropagation();
          emitChange(!val);
        }}
        style={{
          width: 12, height: 12, borderRadius: 2,
          border: `1.5px solid ${color}`,
          background: val ? color : 'transparent',
          flexShrink: 0,
          cursor: readonly ? 'default' : 'pointer',
        }}
      />
    );
  }

  // ── No value yet: show placeholder to set one ──
  if (val === undefined || val === null) {
    if (readonly) return null;
    return (
      <span
        onClick={(e) => { e.stopPropagation(); setEditing(true); }}
        style={{
          fontSize: 9,
          color: '#81928e',
          cursor: 'pointer',
          fontStyle: 'italic',
          flexShrink: 0,
        }}
      >
        ...
      </span>
    );
  }

  const display = typeof val === 'object' ? JSON.stringify(val) : String(val);

  // ── Editing mode: inline input ──
  if (editing && !readonly) {
    const isNumber = dt === 'f64' || dt === 'i64' || dt === 'number' || dt === 'float' || dt === 'int';

    return (
      <>
      <input
        ref={inputRef}
        type={isNumber ? 'number' : 'text'}
        list={suggestions?.length ? `${nodeId}-${pin.name}-options` : undefined}
        defaultValue={display === '""' ? '' : display}
        onChange={(e) => onPreview?.(e.target.value)}
        onBlur={(e) => {
          setEditing(false);
          const raw = e.target.value;
          if (isNumber) {
            const n = dt === 'i64' || dt === 'int' ? parseInt(raw, 10) : parseFloat(raw);
            if (!isNaN(n)) emitChange(n);
          } else {
            // Try parse as JSON for arrays/objects
            try {
              const parsed = JSON.parse(raw);
              emitChange(parsed);
            } catch {
              emitChange(raw);
            }
          }
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
          if (e.key === 'Escape') setEditing(false);
          e.stopPropagation();
        }}
        onClick={(e) => e.stopPropagation()}
        style={{
          fontSize: 10,
          color: '#3c514e',
          background: '#fffefa',
          border: `1px solid ${color}`,
          borderRadius: 3,
          padding: '0 4px',
          lineHeight: '16px',
          width: suggestions?.length ? 92 : 60,
          outline: 'none',
          flexShrink: 0,
          fontFamily: 'monospace',
        }}
      />
      {suggestions?.length ? (
        <datalist id={`${nodeId}-${pin.name}-options`}>
          {suggestions.map((suggestion) => <option key={suggestion} value={suggestion} />)}
        </datalist>
      ) : null}
      </>
    );
  }

  // ── Display mode: clickable badge ──
  if (!display || display === '""' || display === "''") return null;

  return (
    <span
      onClick={(e) => {
        if (readonly) return;
        e.stopPropagation();
        setEditing(true);
      }}
      title={`${pin.name} = ${display} (click to edit)`}
      style={{
        fontSize: 10,
        color: '#718781',
        background: '#f5f7f3',
        border: '1px solid #d6e0dd',
        borderRadius: 3,
        padding: '0 4px',
        lineHeight: '16px',
        maxWidth: 60,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
        flexShrink: 0,
        cursor: readonly ? 'default' : 'pointer',
      }}
    >
      {display}
    </span>
  );
}

/* ─── single pin row ─────────────────────────────────── */
interface PinRowProps {
  inp?: NodePin;
  out?: NodePin;
  connectedPins: Set<string>;
  nodeId: string;
  onEvent?: (event: EditorEvent) => void;
  readonly?: boolean;
  variableTypes?: Record<string, string>;
  onNamePreview?: (value: string) => void;
}

function PinRow({ inp, out, connectedPins, nodeId, onEvent, readonly, variableTypes, onNamePreview }: PinRowProps) {
  const inpColor  = inp ? (isExecPin(inp.kind) ? EXEC_COLOR : getDataTypeColor(inp.data_type)) : 'transparent';
  const outColor  = out ? (isExecPin(out.kind) ? EXEC_COLOR : getDataTypeColor(out.data_type)) : 'transparent';
  const inpAccent = inp ? (isExecPin(inp.kind) ? EXEC_COLOR : getPinAccentColor(inp.data_type)) : 'transparent';
  const outAccent = out ? (isExecPin(out.kind) ? EXEC_COLOR : getPinAccentColor(out.data_type)) : 'transparent';
  const inpConn   = inp ? connectedPins.has(inp.name) : false;
  const outConn   = out ? connectedPins.has(out.name) : false;
  const pinHandle = (
    pin: NodePin,
    connected: boolean,
    accent: string,
    side: 'left' | 'right',
    type: 'source' | 'target',
  ) => {
    const exec = isExecPin(pin.kind);
    const array = !exec && isArrayDataType(pin.data_type);
    return (
      <Handle
        type={type}
        position={side === 'left' ? Position.Left : Position.Right}
        id={pin.name}
        className="blueprint-pin-handle"
        style={{
          width: array ? 20 : 16,
          height: 16,
          [side]: 1,
          top: '50%',
          transform: 'translateY(-50%)',
        }}
      >
        {exec ? (
          <svg className="blueprint-exec-pin" viewBox="0 0 16 16" aria-hidden="true">
            <path
              d="M2.5 2.5 13 8 2.5 13.5Z"
              fill={connected ? accent : '#fffefa'}
              stroke={accent}
              strokeWidth="2"
              strokeLinejoin="round"
            />
          </svg>
        ) : (
          <span className="blueprint-data-pin" style={{ color: accent }}>
            {array && <span className="blueprint-array-bracket">[</span>}
            <span
              className="blueprint-data-pin-dot"
              style={{
                width: array ? 4 : undefined,
                height: array ? 4 : undefined,
                background: connected ? accent : '#fffefa',
                borderColor: accent,
                boxShadow: connected ? `0 0 0 2px ${accent.replace('0.96', '0.16')}` : undefined,
              }}
            />
            {array && <span className="blueprint-array-bracket">]</span>}
          </span>
        )}
      </Handle>
    );
  };

  return (
    <div style={{
      display: 'grid',
      gridTemplateColumns: '1fr 1fr',
      alignItems: 'center',
      height: PIN_ROW_H,
      position: 'relative',
    }}>
      {/* ── Input side ── */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 5, paddingLeft: 21 }}>
        {inp && (
          <>
            {pinHandle(inp, inpConn, inpAccent, 'left', 'target')}
            <span
              title={inp.description ? `${inp.description}\n类型: ${inp.data_type}` : `类型: ${inp.data_type}`}
              style={{
                color: inpColor,
                fontSize: 11,
                fontFamily: '"Segoe UI", sans-serif',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                maxWidth: 80,
                lineHeight: 1,
                cursor: 'default',
              }}>
              {inp.name}
            </span>
            {/* editable default value only when not connected */}
            {!inpConn && inp.kind === 'DataInput' && (
              <DefaultValueWidget
                pin={inp}
                nodeId={nodeId}
                onEvent={onEvent}
                readonly={readonly}
                suggestions={inp.name === 'Name' ? Object.keys(variableTypes ?? {}) : undefined}
                onPreview={inp.name === 'Name' ? onNamePreview : undefined}
              />
            )}
          </>
        )}
      </div>

      {/* ── Output side ── */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 5, paddingRight: 21, justifyContent: 'flex-end' }}>
        {out && (
          <>
            <span
              title={out.description ? `${out.description}\n类型: ${out.data_type}` : `类型: ${out.data_type}`}
              style={{
                color: outColor,
                fontSize: 11,
                fontFamily: '"Segoe UI", sans-serif',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                maxWidth: 80,
                lineHeight: 1,
                textAlign: 'right',
                cursor: 'default',
              }}>
              {out.name}
              {out.name === 'Value' && out.data_type !== 'Any' ? ` · ${out.data_type}` : ''}
            </span>
            {pinHandle(out, outConn, outAccent, 'right', 'source')}
          </>
        )}
      </div>
    </div>
  );
}

/* ─── Blueprint Node ─────────────────────────────────── */
interface BlueprintNodeData {
  nodeType: string;
  displayName?: string;
  description?: string;
  category?: string;
  pins: NodePin[];
  comment?: string;
  size?: { width?: number; height?: number };
  isPure?: boolean;
  connectedPins?: Set<string>;
  onEvent?: (event: EditorEvent) => void;
  readonly?: boolean;
  variableTypes?: Record<string, string>;
  [key: string]: unknown;
}

export function BlueprintNode({ id, data, selected }: NodeProps) {
  const d = data as unknown as BlueprintNodeData;
  const headerColor   = getCategoryColor(d.category ?? 'Default');
  const connectedPins = d.connectedPins ?? new Set<string>();
  const isPure        = d.isPure ?? false;
  const namePin = d.pins.find((pin) => pin.kind === 'DataInput' && pin.name === 'Name');
  const [namePreview, setNamePreview] = useState(
    typeof namePin?.default_value === 'string' ? namePin.default_value : ''
  );
  useEffect(() => {
    setNamePreview(typeof namePin?.default_value === 'string' ? namePin.default_value : '');
  }, [id, namePin?.default_value]);

  const resolvedPins = useMemo(() => {
    if (d.nodeType !== 'GetVarNode') return d.pins;
    const resolvedType = d.variableTypes?.[namePreview] || 'Any';
    return d.pins.map((pin) =>
      pin.kind === 'DataOutput' && pin.name === 'Value'
        ? { ...pin, data_type: resolvedType, resolved_type: resolvedType }
        : pin
    );
  }, [d.nodeType, d.pins, d.variableTypes, namePreview]);

  const inputPins  = [
    ...resolvedPins.filter((p) => p.kind === 'ExecInput'),
    ...resolvedPins.filter((p) => p.kind === 'DataInput'),
  ];
  const outputPins = [
    ...resolvedPins.filter((p) => p.kind === 'ExecOutput'),
    ...resolvedPins.filter((p) => p.kind === 'DataOutput'),
  ];
  const maxPins    = Math.max(inputPins.length, outputPins.length, 1);
  const estimatedWidth = estimateNodeWidth(inputPins, outputPins);
  const nodeWidth = Number.isFinite(d.size?.width) && Number(d.size?.width) > 0 ? Number(d.size?.width) : estimatedWidth;
  const nodeHeight = Number.isFinite(d.size?.height) && Number(d.size?.height) > 0 ? Number(d.size?.height) : undefined;

  return (
    <div style={{
      width: nodeWidth,
      minHeight: nodeHeight,
      background: isPure
        ? 'linear-gradient(180deg, #f2fbf5 0%, #e8f5ee 100%)'
        : 'linear-gradient(180deg, #fffefa 0%, #f8f7f2 100%)',
      borderRadius: 7,
      overflow: 'visible',
      border: selected
        ? '1.5px solid #167d71'
        : isPure
          ? '1.5px solid rgba(34,197,94,0.25)'
          : '1.5px solid #bfd0cb',
      boxShadow: selected
        ? '0 0 0 2px #167d7133, 0 8px 24px rgba(73,104,97,0.2)'
        : '0 4px 18px rgba(73,104,97,0.14)',
      fontFamily: '"Segoe UI", sans-serif',
      position: 'relative',
      transition: 'box-shadow 0.15s ease, border-color 0.15s ease',
    }}>
      {/* ── Header ── */}
      <div style={{
        height: HEADER_HEIGHT,
        background: isPure
          ? `linear-gradient(135deg, #16a34aee, #15803d99)`
          : `linear-gradient(135deg, ${headerColor}ee, ${headerColor}99)`,
        borderRadius: '5px 5px 0 0',
        display: 'flex',
        alignItems: 'center',
        padding: '0 10px',
        gap: 6,
        borderBottom: isPure
          ? '1px solid #16a34a55'
          : `1px solid ${headerColor}55`,
        position: 'relative',
      }}>
        <div style={{
          width: 7, height: 7, borderRadius: '50%',
          background: '#ffffffbb', flexShrink: 0,
        }} />
        <span style={{
          color: '#fff',
          fontWeight: 700,
          fontSize: 12,
          letterSpacing: '0.03em',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          flex: 1,
        }}>
          {d.displayName ?? d.nodeType}
        </span>
        {/* Pure function badge — UE5 "=" mark */}
        {isPure && (
          <span style={{
            fontSize: 11,
            fontWeight: 700,
            color: '#4ade80',
            background: 'rgba(0,0,0,0.35)',
            borderRadius: 3,
            padding: '0 4px',
            lineHeight: '16px',
            flexShrink: 0,
          }}>
            =
          </span>
        )}
      </div>

      {/* ── Description (only when template syntax present) ── */}
      {d.description && d.description.includes('{{') && (
        <div style={{
          fontSize: 10,
          padding: '2px 10px',
            color: '#718781',
            borderBottom: '1px solid #e3e9e6',
          lineHeight: 1.5,
        }}>
          {renderDescription(d.description)}
        </div>
      )}

      {/* ── Pin rows ── */}
      <div style={{ padding: '2px 0' }}>
        {Array.from({ length: maxPins }).map((_, i) => (
          <PinRow
            key={i}
            inp={inputPins[i]}
            out={outputPins[i]}
            connectedPins={connectedPins}
            nodeId={id}
            onEvent={d.onEvent}
            readonly={d.readonly}
            variableTypes={d.variableTypes}
            onNamePreview={d.nodeType === 'GetVarNode' ? setNamePreview : undefined}
          />
        ))}
      </div>

      {/* ── Comment footer ── */}
      {d.comment && (
        <div style={{
          padding: '4px 10px 6px',
          color: '#718781',
          fontSize: 10,
          borderTop: '1px solid #e3e9e6',
          fontStyle: 'italic',
          lineHeight: 1.4,
        }}>
          {d.comment}
        </div>
      )}
    </div>
  );
}
