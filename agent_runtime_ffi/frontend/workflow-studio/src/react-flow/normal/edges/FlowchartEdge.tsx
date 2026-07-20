import React from 'react';
import { BaseEdge, getSmoothStepPath, EdgeLabelRenderer, type EdgeProps } from '@xyflow/react';

/**
 * 流程图边 — 直角折线（smoothstep）。
 *
 * 之所以不用 bezier：
 *   - 流程图惯例用直角线（vs 蓝图惯例用曲线）
 *   - 当 source/target 方向不一致时（decision 的 yes/no），smoothstep 自动绕开，bezier 会画 S 形
 */
export function FlowchartEdge(props: EdgeProps) {
  const {
    sourceX, sourceY, targetX, targetY,
    sourcePosition, targetPosition,
    label, markerEnd,
  } = props;

  const [edgePath, labelX, labelY] = getSmoothStepPath({
    sourceX, sourceY, targetX, targetY,
    sourcePosition, targetPosition,
    borderRadius: 8,
  });

  // yes/否/循环体 不同标签着色，让普通用户一眼看清分支语义
  const labelStr = label != null ? String(label) : '';
  let labelBg = '#ffffff';
  let labelColor = '#64748b';
  let labelBorder = '#e2e8f0';
  if (labelStr === '是' || labelStr.toLowerCase() === 'yes') {
    labelBg = '#dcfce7'; labelColor = '#15803d'; labelBorder = '#86efac';
  } else if (labelStr === '否' || labelStr.toLowerCase() === 'no') {
    labelBg = '#fee2e2'; labelColor = '#b91c1c'; labelBorder = '#fca5a5';
  } else if (labelStr === '循环体' || labelStr === '每个元素') {
    labelBg = '#ede9fe'; labelColor = '#6d28d9'; labelBorder = '#c4b5fd';
  }

  return (
    <>
      {/* shadow */}
      <BaseEdge path={edgePath} style={{ stroke: '#cbd5e1', strokeWidth: 4, opacity: 0.25 }} />
      {/* main */}
      <BaseEdge path={edgePath} markerEnd={markerEnd}
        style={{ stroke: '#94a3b8', strokeWidth: 1.8 }} />

      {label && (
        <EdgeLabelRenderer>
          <div
            style={{
              position: 'absolute',
              transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
              pointerEvents: 'none',
              background: labelBg,
              border: `1px solid ${labelBorder}`,
              borderRadius: 999,
              padding: '2px 8px',
              fontSize: 11,
              fontWeight: 700,
              color: labelColor,
              fontFamily: '"Segoe UI", system-ui, sans-serif',
              boxShadow: '0 1px 4px rgba(0,0,0,0.08)',
              whiteSpace: 'nowrap',
              letterSpacing: 0.3,
            }}
          >
            {labelStr}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
}
