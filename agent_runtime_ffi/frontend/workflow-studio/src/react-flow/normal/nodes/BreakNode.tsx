import React from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { NODE_COLORS } from '../../theme';
import { ArrowRightCircle } from 'lucide-react';

const C = NODE_COLORS.break;

export function BreakNode({ data }: NodeProps) {
  return (
    <div style={{
      background: '#fef2f2',
      borderRadius: 8,
      padding: '6px 12px',
      fontSize: 12,
      fontWeight: 700,
      fontFamily: '"Segoe UI", system-ui, sans-serif',
      color: C,
      border: `1.5px solid ${C}66`,
      borderLeft: `3px solid ${C}`,
      boxShadow: `0 2px 8px ${C}22`,
      whiteSpace: 'nowrap',
      letterSpacing: '0.02em',
      display: 'flex',
      alignItems: 'center',
      gap: 6,
    }}>
      <Handle type="target" position={Position.Top}
        style={{ background: C, border: '2px solid #fff', width: 10, height: 10 }} />
      <ArrowRightCircle size={12} strokeWidth={2.4} />
      {(data as any).label ?? '跳出循环'}
    </div>
  );
}
