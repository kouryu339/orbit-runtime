import React from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { NODE_COLORS } from '../../theme';
import { Play } from 'lucide-react';

const C = NODE_COLORS.start;

interface StartData {
  label?: string;
  detail?: string;
}

export function StartNode({ data }: NodeProps) {
  const d = data as unknown as StartData;
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
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <div style={{
          width: 24,
          height: 24,
          borderRadius: '50%',
          background: `linear-gradient(135deg, ${C}, #16a34a)`,
          color: '#fff',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexShrink: 0,
        }}>
          <Play size={12} strokeWidth={2.5} fill="#fff" />
        </div>
        <span style={{
          fontSize: 13,
          fontWeight: 700,
          color: '#14532d',
          letterSpacing: '0.02em',
        }}>
          {d.label ?? '开始'}
        </span>
      </div>

      {d.detail && (
        <div style={{
          marginTop: 6,
          paddingTop: 6,
          borderTop: '1px dashed #d1fae5',
          fontSize: 11,
          color: '#15803d',
          lineHeight: 1.4,
          fontFamily: '"Cascadia Code", "Consolas", monospace',
        }}>
          {d.detail}
        </div>
      )}

      <Handle type="source" position={Position.Bottom}
        style={{ background: C, border: '2px solid #fff', width: 10, height: 10 }} />
    </div>
  );
}
