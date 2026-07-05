/**
 * 节点共享：徽章、底部数据流标签、状态视觉。
 * 抽出来避免每个节点组件重复 50 行。
 */
import React from 'react';
import { CheckCircle2, AlertCircle, Loader2 } from 'lucide-react';
import type { FlowchartStepStatus } from '../types';

/** 步骤编号徽章 — `#1` / `#1.1` */
export function StepBadge({ stepId }: { stepId?: string }) {
  if (!stepId) return null;
  return (
    <span style={{
      display: 'inline-flex',
      alignItems: 'center',
      fontSize: 10,
      color: '#94a3b8',
      fontWeight: 500,
      background: '#f1f5f9',
      padding: '1px 6px',
      borderRadius: 999,
      marginRight: 6,
      letterSpacing: 0.3,
      lineHeight: 1.4,
    }}>
      #{stepId}
    </span>
  );
}

/** 数据流来源行：← input.url, ← 步骤1.title */
export function DataFromRow({ items }: { items?: string[] }) {
  if (!items || items.length === 0) return null;
  const text = items.join('  ');
  return (
    <div
      title={items.join('\n')}
      style={{
        fontSize: 10,
        color: '#94a3b8',
        marginTop: 4,
        padding: '0 14px',
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        fontFamily: '"Cascadia Code", "Consolas", monospace',
      }}
    >
      ← {text}
    </div>
  );
}

/** 输出引脚行：→ Result, page, html */
export function OutputsRow({ items }: { items?: string[] }) {
  if (!items || items.length === 0) return null;
  const text = items.join(', ');
  return (
    <div
      title={`输出: ${items.join(', ')}`}
      style={{
        fontSize: 10,
        color: '#64748b',
        marginTop: 2,
        padding: '0 14px',
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        fontFamily: '"Cascadia Code", "Consolas", monospace',
      }}
    >
      → {text}
    </div>
  );
}

/** 状态指示图标 — 节点右上角小圆 */
export function StatusBadge({ status }: { status?: FlowchartStepStatus }) {
  if (!status || status === 'pending') return null;

  const presets = {
    running:   { icon: Loader2,       color: '#3b82f6', bg: '#dbeafe', spin: true,  title: '执行中' },
    completed: { icon: CheckCircle2,  color: '#22c55e', bg: '#dcfce7', spin: false, title: '已完成' },
    error:     { icon: AlertCircle,   color: '#ef4444', bg: '#fee2e2', spin: false, title: '出错' },
  } as const;
  const p = presets[status];
  const Icon = p.icon;

  return (
    <div
      title={p.title}
      style={{
        position: 'absolute',
        top: -6,
        right: -6,
        width: 18,
        height: 18,
        borderRadius: '50%',
        background: p.bg,
        border: `1.5px solid ${p.color}`,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 10,
        boxShadow: `0 2px 8px ${p.color}55`,
      }}
    >
      <Icon size={10} color={p.color} style={p.spin ? { animation: 'fc-spin 1s linear infinite' } : undefined} />
    </div>
  );
}

/** 根据状态返回节点边框色 + 阴影色（覆盖默认） */
export function statusOverlay(status?: FlowchartStepStatus): {
  borderColor?: string;
  boxShadow?: string;
  animation?: string;
} {
  switch (status) {
    case 'running':
      return {
        borderColor: '#3b82f6',
        boxShadow: '0 0 0 3px rgba(59,130,246,0.15), 0 4px 16px rgba(59,130,246,0.3)',
        animation: 'fc-pulse 2s ease-in-out infinite',
      };
    case 'completed':
      return {
        borderColor: '#22c55e',
        boxShadow: '0 2px 12px rgba(34,197,94,0.25)',
      };
    case 'error':
      return {
        borderColor: '#ef4444',
        boxShadow: '0 0 0 3px rgba(239,68,68,0.15), 0 4px 16px rgba(239,68,68,0.3)',
      };
    default:
      return {};
  }
}

/** 全局 keyframes — 在 NormalFlowEditor 注入一次即可 */
export const KEYFRAMES_CSS = `
@keyframes fc-spin {
  to { transform: rotate(360deg); }
}
@keyframes fc-pulse {
  0%, 100% { box-shadow: 0 0 0 3px rgba(59,130,246,0.15), 0 4px 16px rgba(59,130,246,0.3); }
  50%      { box-shadow: 0 0 0 6px rgba(59,130,246,0.10), 0 6px 20px rgba(59,130,246,0.4); }
}
`;
