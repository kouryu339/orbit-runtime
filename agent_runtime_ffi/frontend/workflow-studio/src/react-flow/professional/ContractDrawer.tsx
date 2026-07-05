/**
 * 契约抽屉 — 右侧固定面板，编辑工作流的 INPUT / RETURN / Variables。
 *
 * 数据来源：BlueprintJson
 *   - INPUT  ← StartNode 的 DataInput/DataOutput 引脚对（同名）
 *   - RETURN ← EndNode   的 DataInput 引脚
 *   - VAR    ← BlueprintJson.variables（注：当前后端未填充，预留接口）
 *
 * 事件输出：通过 onEvent 触发 inputdeclare/inputupdate/...，
 *           workflow-editor.component 转发到 WorkflowDraftService 的 declare* / updateContract / removeContract / renameContract。
 *
 * 设计原则：
 *   - 每个契约项一行，name 双击改名，type 下拉，default 可编辑
 *   - 删除有二次确认，避免误触
 *   - readonly 模式下隐藏所有编辑按钮
 */
import React, { useMemo, useState } from 'react';
import type { BlueprintJson, BlueprintNodeJson, EditorEvent, NodePin } from '../types';
import { getDataTypeColor, getPinAccentColor, isArrayDataType } from '../theme';

const PANEL_BG = '#fffefa';
const PANEL_BORDER = '#cbd9d5';
const SECTION_BG = '#f7f5ef';
const TEXT_PRIMARY = '#304944';
const TEXT_SECONDARY = '#718781';
const TEXT_MUTED = '#9aaba6';
const ACCENT = '#167d71';
const DANGER = '#ef4444';

const COMMON_TYPES = [
  'Number',
  'String',
  'bool',
  'Array<Number>',
  'Array<String>',
  'Array<bool>',
  'Any',
];

interface Props {
  blueprint: BlueprintJson | null;
  onEvent?: (event: EditorEvent) => void;
  readonly?: boolean;
}

interface ContractRow {
  name: string;
  dataType: string;
  defaultValue?: unknown;
  /** input 同名引脚的 default_value 在 DataInput 上 */
  hasDefault: boolean;
}

/** 从 StartNode 提取 INPUT 契约：每个 DataOutput 引脚对应一项，default 来自同名 DataInput */
function extractInputs(start: BlueprintNodeJson | undefined): ContractRow[] {
  if (!start) return [];
  const dataOuts = start.pins.filter((p) => p.kind === 'DataOutput');
  return dataOuts.map((p) => {
    const di = start.pins.find((x) => x.name === p.name && x.kind === 'DataInput');
    return {
      name: p.name,
      dataType: p.data_type || 'Any',
      defaultValue: di?.default_value,
      hasDefault: di?.default_value !== undefined && di?.default_value !== null,
    };
  });
}

/** 从 EndNode 提取 RETURN 契约：每个 DataInput 引脚对应一项 */
function extractReturns(end: BlueprintNodeJson | undefined): ContractRow[] {
  if (!end) return [];
  return end.pins
    .filter((p) => p.kind === 'DataInput')
    .map((p) => ({
      name: p.name,
      dataType: p.data_type || 'Any',
      defaultValue: p.default_value,
      hasDefault: p.default_value !== undefined && p.default_value !== null,
    }));
}

/** 渲染默认值的紧凑字符串形式 */
function formatDefault(v: unknown): string {
  if (v === undefined || v === null) return '';
  if (typeof v === 'string') return `"${v}"`;
  if (typeof v === 'object') return JSON.stringify(v);
  return String(v);
}

/** 解析编辑器输入回 unknown（按目标类型尝试 number/bool） */
function parseDefault(raw: string, dataType: string): unknown {
  const s = raw.trim();
  if (s === '') return null;
  if (isArrayDataType(dataType) || dataType === 'Object' || dataType === 'Any') {
    try {
      return JSON.parse(s);
    } catch {
      if (isArrayDataType(dataType) || dataType === 'Object') return s;
    }
  }
  if (dataType === 'i64') {
    const n = parseInt(s, 10);
    return Number.isFinite(n) ? n : s;
  }
  if (dataType === 'f64' || dataType === 'Number') {
    const n = parseFloat(s);
    return Number.isFinite(n) ? n : s;
  }
  if (dataType === 'bool' || dataType === 'Boolean') {
    return s === 'true' || s === '1';
  }
  // String/Path/Date/Time/Any: 去掉首尾引号
  if ((s.startsWith('"') && s.endsWith('"')) || (s.startsWith("'") && s.endsWith("'"))) {
    return s.slice(1, -1);
  }
  return s;
}

/* ─── 单行 ────────────────────────────────────────────── */

interface RowProps {
  row: ContractRow;
  kind: 'input' | 'return' | 'var';
  readonly?: boolean;
  onUpdate: (name: string, patch: { dataType?: string; defaultValue?: unknown }) => void;
  onRename: (oldName: string, newName: string) => void;
  onRemove: (name: string) => void;
}

function ContractRowView({ row, kind, readonly, onUpdate, onRename, onRemove }: RowProps) {
  const [editingName, setEditingName] = useState(false);
  const [nameDraft, setNameDraft] = useState(row.name);
  const [defaultDraft, setDefaultDraft] = useState(formatDefault(row.defaultValue));
  const color = getDataTypeColor(row.dataType);
  const accent = getPinAccentColor(row.dataType);

  // 同步外部更新
  React.useEffect(() => {
    if (!editingName) setNameDraft(row.name);
  }, [row.name, editingName]);
  React.useEffect(() => {
    setDefaultDraft(formatDefault(row.defaultValue));
  }, [row.defaultValue]);

  const commitName = () => {
    setEditingName(false);
    const next = nameDraft.trim();
    if (!next || next === row.name) {
      setNameDraft(row.name);
      return;
    }
    onRename(row.name, next);
  };

  return (
    <div
      style={{
        position: 'relative',
        display: 'grid',
        gridTemplateColumns: 'minmax(0, 1fr) 104px',
        gap: 6,
        alignItems: 'end',
        minWidth: 0,
        padding: '8px 36px 8px 8px',
        background: SECTION_BG,
        borderRadius: 4,
        marginBottom: 6,
        fontSize: 12,
        fontFamily: '"Cascadia Code", "Consolas", monospace',
      }}
    >
      {/* name */}
      {editingName ? (
        <input
          autoFocus
          value={nameDraft}
          onChange={(e) => setNameDraft(e.target.value)}
          onBlur={commitName}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commitName();
            if (e.key === 'Escape') {
              setNameDraft(row.name);
              setEditingName(false);
            }
          }}
          style={{
            width: '100%',
            minWidth: 0,
            boxSizing: 'border-box',
            background: '#eef4f1',
            border: `1px solid ${ACCENT}`,
            color: TEXT_PRIMARY,
            padding: '2px 4px',
            borderRadius: 3,
            fontSize: 12,
            fontFamily: 'inherit',
          }}
        />
      ) : (
        <span
          title={readonly ? row.name : '双击重命名'}
          onDoubleClick={() => !readonly && setEditingName(true)}
          style={{
            minWidth: 0,
            color: TEXT_PRIMARY,
            cursor: readonly ? 'default' : 'text',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {row.name}
        </span>
      )}

      {/* type */}
      <select
        disabled={readonly}
        value={row.dataType}
        onChange={(e) => onUpdate(row.name, { dataType: e.target.value })}
        style={{
          width: '100%',
          minWidth: 0,
          boxSizing: 'border-box',
          background: '#eef4f1',
          border: `1px solid ${accent}55`,
          color,
          padding: '2px 4px',
          borderRadius: 3,
          fontSize: 11,
          fontFamily: 'inherit',
        }}
      >
        {COMMON_TYPES.includes(row.dataType) ? null : <option value={row.dataType}>{row.dataType}</option>}
        {COMMON_TYPES.map((t) => (
          <option key={t} value={t}>
            {t}
          </option>
        ))}
      </select>

      {kind === 'return' ? (
        <div
          style={{
            gridColumn: '1 / -1',
            color: TEXT_MUTED,
            fontSize: 10,
            fontFamily: '"Segoe UI", sans-serif',
          }}
        >
          Value supplied by workflow connections
        </div>
      ) : (
        <input
          disabled={readonly}
          value={defaultDraft}
          placeholder="Default value"
          onChange={(e) => setDefaultDraft(e.target.value)}
          onBlur={() => {
            const parsed = parseDefault(defaultDraft, row.dataType);
            if (formatDefault(parsed) !== formatDefault(row.defaultValue)) {
              onUpdate(row.name, { defaultValue: parsed });
            }
          }}
          style={{
            gridColumn: '1 / -1',
            width: '100%',
            minWidth: 0,
            boxSizing: 'border-box',
            background: '#eef4f1',
            border: `1px solid ${PANEL_BORDER}`,
            color: TEXT_SECONDARY,
            padding: '5px 6px',
            borderRadius: 3,
            fontSize: 11,
            fontFamily: 'inherit',
          }}
        />
      )}

      {/* delete */}
      {!readonly && (
        <button
          title="删除"
          onClick={() => {
            if (confirm(`确定删除 ${kind.toUpperCase()} ${row.name}？\n脚本里的引用会变成未定义。`)) {
              onRemove(row.name);
            }
          }}
          style={{
            position: 'absolute',
            top: 8,
            right: 8,
            background: 'transparent',
            border: 'none',
            color: TEXT_MUTED,
            cursor: 'pointer',
            padding: 0,
            fontSize: 14,
            lineHeight: 1,
          }}
          onMouseEnter={(e) => (e.currentTarget.style.color = DANGER)}
          onMouseLeave={(e) => (e.currentTarget.style.color = TEXT_MUTED)}
        >
          ×
        </button>
      )}
    </div>
  );
}

/* ─── 章节 ────────────────────────────────────────────── */

interface SectionProps {
  title: string;
  kind: 'input' | 'return' | 'var';
  rows: ContractRow[];
  readonly?: boolean;
  onAdd: (name: string, dataType: string) => void;
  onUpdate: (name: string, patch: { dataType?: string; defaultValue?: unknown }) => void;
  onRename: (oldName: string, newName: string) => void;
  onRemove: (name: string) => void;
  emptyHint: string;
}

function Section({ title, kind, rows, readonly, onAdd, onUpdate, onRename, onRemove, emptyHint }: SectionProps) {
  const [adding, setAdding] = useState(false);
  const [newName, setNewName] = useState('');
  const [newType, setNewType] = useState('String');
  const newTypeAccent = getPinAccentColor(newType);

  const submitNew = () => {
    const n = newName.trim();
    if (!n) {
      setAdding(false);
      return;
    }
    onAdd(n, newType);
    setNewName('');
    setNewType('String');
    setAdding(false);
  };

  return (
    <div style={{ marginBottom: 16 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: 6,
          padding: '0 4px',
        }}
      >
        <div style={{ color: TEXT_PRIMARY, fontSize: 12, fontWeight: 600, letterSpacing: 0.5 }}>
          {title} <span style={{ color: TEXT_MUTED, fontWeight: 400 }}>({rows.length})</span>
        </div>
        {!readonly && (
          <button
            title={`新增 ${kind}`}
            onClick={() => setAdding(true)}
            style={{
              background: 'transparent',
              border: `1px solid ${PANEL_BORDER}`,
              color: TEXT_SECONDARY,
              cursor: 'pointer',
              fontSize: 11,
              padding: '2px 8px',
              borderRadius: 3,
            }}
            onMouseEnter={(e) => (e.currentTarget.style.borderColor = ACCENT)}
            onMouseLeave={(e) => (e.currentTarget.style.borderColor = PANEL_BORDER)}
          >
            + 添加
          </button>
        )}
      </div>

      {rows.length === 0 && !adding && (
        <div
          style={{
            color: TEXT_MUTED,
            fontSize: 11,
            fontStyle: 'italic',
            padding: '8px',
            textAlign: 'center',
          }}
        >
          {emptyHint}
        </div>
      )}

      {rows.map((r) => (
        <ContractRowView
          key={r.name}
          row={r}
          kind={kind}
          readonly={readonly}
          onUpdate={onUpdate}
          onRename={onRename}
          onRemove={onRemove}
        />
      ))}

      {adding && (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'minmax(0, 1fr) auto auto',
            gap: 6,
            minWidth: 0,
            padding: '6px 8px',
            background: '#eef4f1',
            borderRadius: 4,
            fontSize: 12,
          }}
        >
          <input
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder="名称"
            onKeyDown={(e) => {
              if (e.key === 'Enter') submitNew();
              if (e.key === 'Escape') setAdding(false);
            }}
            style={{
              gridColumn: '1 / -1',
              width: '100%',
              minWidth: 0,
              boxSizing: 'border-box',
              background: '#fffefa',
              border: `1px solid ${ACCENT}`,
              color: TEXT_PRIMARY,
              padding: '2px 4px',
              borderRadius: 3,
              fontSize: 12,
              fontFamily: '"Cascadia Code", "Consolas", monospace',
            }}
          />
          <select
            value={newType}
            onChange={(e) => setNewType(e.target.value)}
            style={{
              width: '100%',
              minWidth: 0,
              boxSizing: 'border-box',
              background: '#fffefa',
              border: `1px solid ${newTypeAccent}55`,
              color: getDataTypeColor(newType),
              padding: '2px 4px',
              borderRadius: 3,
              fontSize: 11,
            }}
          >
            {COMMON_TYPES.map((type) => (
              <option key={type} value={type}>
                {type}
              </option>
            ))}
          </select>
          <button
            onClick={submitNew}
            style={{
              background: ACCENT,
              border: 'none',
              color: '#fff',
              padding: '2px 8px',
              borderRadius: 3,
              cursor: 'pointer',
              fontSize: 11,
            }}
          >
            ✓
          </button>
          <button
            onClick={() => setAdding(false)}
            style={{
              background: 'transparent',
              border: `1px solid ${PANEL_BORDER}`,
              color: TEXT_MUTED,
              padding: '2px 6px',
              borderRadius: 3,
              cursor: 'pointer',
              fontSize: 11,
            }}
          >
            ×
          </button>
        </div>
      )}
    </div>
  );
}

/* ─── 主组件 ───────────────────────────────────────────── */

export function ContractDrawer({ blueprint, onEvent, readonly }: Props) {
  const [collapsed, setCollapsed] = useState(false);

  const { inputs, returns, vars } = useMemo(() => {
    if (!blueprint) return { inputs: [] as ContractRow[], returns: [] as ContractRow[], vars: [] as ContractRow[] };
    const start = blueprint.nodes.find((n) => n.node_type === 'StartNode');
    const end = blueprint.nodes.find((n) => n.node_type === 'EndNode');
    const vs: ContractRow[] = (blueprint.variables ?? []).map((v) => ({
      name: v.name,
      dataType: v.data_type || 'Any',
      defaultValue: v.default_value,
      hasDefault: v.default_value !== undefined && v.default_value !== null,
    }));
    return { inputs: extractInputs(start), returns: extractReturns(end), vars: vs };
  }, [blueprint]);

  if (collapsed) {
    return (
      <button
        title="展开契约面板"
        onClick={() => setCollapsed(false)}
        style={{
          position: 'absolute',
          top: 12,
          right: 12,
          zIndex: 50,
          background: PANEL_BG,
          border: `1px solid ${PANEL_BORDER}`,
          color: TEXT_SECONDARY,
          padding: '6px 10px',
          borderRadius: 4,
          cursor: 'pointer',
          fontSize: 11,
        }}
      >
        契约 ({inputs.length}/{returns.length}/{vars.length})
      </button>
    );
  }

  return (
    <div
      style={{
        position: 'absolute',
        top: 0,
        right: 0,
        bottom: 0,
        width: 'min(380px, calc(100% - 24px))',
        maxWidth: '100%',
        overflow: 'hidden',
        background: PANEL_BG,
        borderLeft: `1px solid ${PANEL_BORDER}`,
        boxShadow: '-4px 0 16px rgba(73,104,97,0.16)',
        display: 'flex',
        flexDirection: 'column',
        zIndex: 50,
        fontFamily: '"Segoe UI", sans-serif',
      }}
    >
      {/* 头 */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '10px 12px',
          borderBottom: `1px solid ${PANEL_BORDER}`,
        }}
      >
        <div style={{ color: TEXT_PRIMARY, fontSize: 13, fontWeight: 600 }}>工作流契约</div>
        <button
          onClick={() => setCollapsed(true)}
          title="收起"
          style={{
            background: 'transparent',
            border: 'none',
            color: TEXT_MUTED,
            cursor: 'pointer',
            fontSize: 16,
            padding: 0,
            lineHeight: 1,
          }}
        >
          →
        </button>
      </div>

      {/* 内容（可滚动） */}
      <div style={{ flex: 1, minWidth: 0, overflowX: 'hidden', overflowY: 'auto', padding: 12 }}>
        {!blueprint && (
          <div style={{ color: TEXT_MUTED, fontSize: 11, textAlign: 'center', padding: 24 }}>
            暂无草稿
          </div>
        )}

        {blueprint && (
          <>
            <Section
              title="INPUT (入参)"
              kind="input"
              rows={inputs}
              readonly={readonly}
              emptyHint="尚未声明任何入参"
              onAdd={(name, dataType) => onEvent?.({ type: 'inputdeclare', payload: { name, dataType } })}
              onUpdate={(name, patch) =>
                onEvent?.({ type: 'inputupdate', payload: { name, ...patch } })
              }
              onRename={(oldName, newName) =>
                onEvent?.({ type: 'inputrename', payload: { oldName, newName } })
              }
              onRemove={(name) => onEvent?.({ type: 'inputremove', payload: { name } })}
            />

            <Section
              title="RETURN (出参)"
              kind="return"
              rows={returns}
              readonly={readonly}
              emptyHint="尚未声明任何出参"
              onAdd={(name, dataType) => onEvent?.({ type: 'returndeclare', payload: { name, dataType } })}
              onUpdate={(name, patch) =>
                onEvent?.({ type: 'returnupdate', payload: { name, ...patch } })
              }
              onRename={(oldName, newName) =>
                onEvent?.({ type: 'returnrename', payload: { oldName, newName } })
              }
              onRemove={(name) => onEvent?.({ type: 'returnremove', payload: { name } })}
            />

            <Section
              title="VAR ($变量)"
              kind="var"
              rows={vars}
              readonly={readonly}
              emptyHint="尚未声明任何可变变量"
              onAdd={(name, dataType) => onEvent?.({ type: 'vardeclare', payload: { name, dataType } })}
              onUpdate={(name, patch) =>
                onEvent?.({ type: 'varupdate', payload: { name, ...patch } })
              }
              onRename={(oldName, newName) =>
                onEvent?.({ type: 'varrename', payload: { oldName, newName } })
              }
              onRemove={(name) => onEvent?.({ type: 'varremove', payload: { name } })}
            />
          </>
        )}
      </div>
    </div>
  );
}
