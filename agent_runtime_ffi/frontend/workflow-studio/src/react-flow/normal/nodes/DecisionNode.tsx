import React from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { NODE_COLORS } from '../../theme';
import { GitBranch } from 'lucide-react';
import { StepBadge, StatusBadge, statusOverlay } from '../nodeShared';
import type { FlowchartStepStatus } from '../../types';

const C = NODE_COLORS.decision;

interface DecisionData {
  label: string;
  step_id?: string;
  status?: FlowchartStepStatus;
}

/**
 * 决策节点 — 不再画 SVG 菱形（label 长度不固定难看），改成色条卡片 + 顶部图标提示。
 * 用 yes/no 两个 source handle，但都从底部出（避免 dagre TB 布局下右出 handle 的 S 形边）。
 */
export function DecisionNode({ data }: NodeProps) {
  const d = data as unknown as DecisionData;
  const overlay = statusOverlay(d.status);

  return (
    <div style={{
      position: 'relative',
      background: '#fffbeb',
      borderRadius: 10,
      minWidth: 180,
      maxWidth: 260,
      boxShadow: overlay.boxShadow ?? `0 2px 12px ${C}33, 0 1px 3px rgba(0,0,0,0.06)`,
      border: `1.5px solid ${overlay.borderColor ?? C}`,
      fontFamily: '"Segoe UI", system-ui, sans-serif',
      overflow: 'hidden',
      transition: 'box-shadow 0.2s, border-color 0.2s',
      animation: overlay.animation,
    }}>
      <Handle type="target" position={Position.Top}
        style={{ background: C, border: '2px solid #fff', width: 10, height: 10 }} />

      <StatusBadge status={d.status} />

      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 10, padding: '10px 14px' }}>
        <div style={{
          flexShrink: 0,
          width: 26,
          height: 26,
          borderRadius: 6,
          background: C,
          color: '#fff',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}>
          <GitBranch size={14} strokeWidth={2.4} />
        </div>
        <div style={{ minWidth: 0, flex: 1, paddingTop: 1 }}>
          <div style={{ fontSize: 13, fontWeight: 700, color: '#78350f', lineHeight: 1.4, wordBreak: 'break-word' }}>
            <StepBadge stepId={d.step_id} />
            {d.label}
          </div>
        </div>
      </div>

      {/* 两个 source handle 都从底部出，由 dagre 控制 yes/no 的水平错位 */}
      <Handle type="source" position={Position.Bottom} id="yes"
        style={{ background: '#22c55e', border: '2px solid #fff', width: 10, height: 10, left: '30%' }} />
      <Handle type="source" position={Position.Bottom} id="no"
        style={{ background: '#ef4444', border: '2px solid #fff', width: 10, height: 10, left: '70%' }} />
    </div>
  );
}
