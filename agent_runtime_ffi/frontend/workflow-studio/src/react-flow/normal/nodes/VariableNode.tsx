import React from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { NODE_COLORS } from '../../theme';
import { Variable } from 'lucide-react';
import { StepBadge, StatusBadge, statusOverlay } from '../nodeShared';
import type { FlowchartStepStatus } from '../../types';

const C = NODE_COLORS.variable;

interface VarData {
  label: string;
  detail?: string;
  step_id?: string;
  status?: FlowchartStepStatus;
}

export function VariableNode({ data }: NodeProps) {
  const d = data as unknown as VarData;
  const overlay = statusOverlay(d.status);

  return (
    <div style={{
      position: 'relative',
      background: '#f0fdfa',
      borderRadius: 8,
      padding: '7px 12px',
      fontSize: 12,
      fontFamily: '"Segoe UI", system-ui, sans-serif',
      color: '#134e4a',
      border: `1px solid ${overlay.borderColor ?? `${C}55`}`,
      borderLeft: `3px solid ${overlay.borderColor ?? C}`,
      boxShadow: overlay.boxShadow ?? '0 1px 6px rgba(0,0,0,0.06)',
      whiteSpace: 'nowrap',
      display: 'flex',
      alignItems: 'center',
      gap: 6,
      minWidth: 140,
      maxWidth: 240,
      animation: overlay.animation,
    }}>
      <Handle type="target" position={Position.Top}
        style={{ background: C, border: '2px solid #fff', width: 10, height: 10 }} />

      <StatusBadge status={d.status} />

      <Variable size={12} color={C} strokeWidth={2.4} style={{ flexShrink: 0 }} />
      <span style={{ fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis' }}>
        <StepBadge stepId={d.step_id} />
        {d.label}
      </span>
      {d.detail && (
        <span style={{ color: '#0f766e', fontSize: 11, fontFamily: '"Cascadia Code", monospace', overflow: 'hidden', textOverflow: 'ellipsis' }}>
          {d.detail}
        </span>
      )}

      <Handle type="source" position={Position.Bottom}
        style={{ background: C, border: '2px solid #fff', width: 10, height: 10 }} />
    </div>
  );
}
