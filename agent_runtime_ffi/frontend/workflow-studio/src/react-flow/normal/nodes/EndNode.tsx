import React from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { NODE_COLORS } from '../../theme';
import { StopCircle } from 'lucide-react';

const C = NODE_COLORS.end;

interface EndData {
  label?: string;
  detail?: string;
}

export function EndNode({ data }: NodeProps) {
  const d = data as unknown as EndData;
  return (
    <div style={{
      background: '#ffffff',
      border: `2px solid ${C}`,
      borderRadius: 12,
      minWidth: 140,
      maxWidth: 240,
      padding: '8px 14px',
      fontFamily: '"Segoe UI", system-ui, sans-serif',
      boxShadow: `0 4px 16px ${C}33`,
      position: 'relative',
    }}>
      <Handle type="target" position={Position.Top}
        style={{ background: C, border: '2px solid #fff', width: 10, height: 10 }} />

      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <div style={{
          width: 24,
          height: 24,
          borderRadius: '50%',
          background: `linear-gradient(135deg, ${C}, #dc2626)`,
          color: '#fff',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexShrink: 0,
        }}>
          <StopCircle size={14} strokeWidth={2.4} />
        </div>
        <span style={{
          fontSize: 13,
          fontWeight: 700,
          color: '#7f1d1d',
          letterSpacing: '0.02em',
        }}>
          {d.label ?? '结束'}
        </span>
      </div>

      {d.detail && (
        <div style={{
          marginTop: 6,
          paddingTop: 6,
          borderTop: '1px dashed #fecaca',
          fontSize: 11,
          color: '#b91c1c',
          lineHeight: 1.4,
          fontFamily: '"Cascadia Code", "Consolas", monospace',
        }}>
          {d.detail}
        </div>
      )}
    </div>
  );
}
