import React from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { NODE_COLORS } from '../../theme';
import { getNodeIcon } from '../icons';
import { StepBadge, DataFromRow, OutputsRow, StatusBadge, statusOverlay } from '../nodeShared';
import type { FlowchartStepStatus } from '../../types';

const C = NODE_COLORS.action;

interface ActionData {
  label: string;
  detail?: string;
  step_id?: string;
  outputs?: string[];
  data_from?: string[];
  kind?: string;
  status?: FlowchartStepStatus;
}

export function ActionNode({ data }: NodeProps) {
  const d = data as unknown as ActionData;
  const Icon = getNodeIcon('action', d.kind);
  const overlay = statusOverlay(d.status);

  return (
    <div style={{
      position: 'relative',
      background: '#ffffff',
      borderRadius: 10,
      minWidth: 180,
      maxWidth: 280,
      boxShadow: overlay.boxShadow ?? '0 2px 12px rgba(0,0,0,0.08), 0 1px 3px rgba(0,0,0,0.06)',
      border: `1px solid ${overlay.borderColor ?? '#e2e8f0'}`,
      borderLeft: `4px solid ${overlay.borderColor ?? C}`,
      fontFamily: '"Segoe UI", system-ui, sans-serif',
      overflow: 'hidden',
      paddingBottom: (d.outputs?.length || d.data_from?.length) ? 6 : 0,
      transition: 'box-shadow 0.2s, border-color 0.2s',
      animation: overlay.animation,
    }}>
      <Handle type="target" position={Position.Top}
        style={{ background: C, border: '2px solid #fff', width: 10, height: 10 }} />

      <StatusBadge status={d.status} />

      {/* 主体：图标 + 标题 */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 10, padding: '10px 14px' }}>
        <div style={{
          flexShrink: 0,
          width: 26,
          height: 26,
          borderRadius: 6,
          background: `${C}15`,
          color: C,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}>
          <Icon size={14} strokeWidth={2.2} />
        </div>
        <div style={{ minWidth: 0, flex: 1, paddingTop: 1 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: '#1e293b', lineHeight: 1.4, wordBreak: 'break-word' }}>
            <StepBadge stepId={d.step_id} />
            {d.label}
          </div>
        </div>
      </div>

      <DataFromRow items={d.data_from} />
      <OutputsRow   items={d.outputs} />

      <Handle type="source" position={Position.Bottom}
        style={{ background: C, border: '2px solid #fff', width: 10, height: 10 }} />
    </div>
  );
}
