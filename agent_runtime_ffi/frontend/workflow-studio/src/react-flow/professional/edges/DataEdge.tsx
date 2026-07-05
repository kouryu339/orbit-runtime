import React from 'react';
import { BaseEdge, getBezierPath, type EdgeProps } from '@xyflow/react';

/**
 * Blueprint-style data wire.
 */
export function DataEdge(props: EdgeProps) {
  const { sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, markerEnd, style, selected } = props;
  const [edgePath] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
    curvature: 0.42,
  });

  const strokeColor = (style as React.CSSProperties | undefined)?.stroke ?? '#6b7280';

  return (
    <>
      <BaseEdge
        path={edgePath}
        style={{
          stroke: 'rgba(255, 254, 250, 0.88)',
          strokeWidth: selected ? 5.2 : 4,
        }}
      />
      <BaseEdge
        path={edgePath}
        markerEnd={markerEnd}
        style={{
          ...style,
          stroke: strokeColor,
          strokeWidth: selected ? 2.8 : 2,
          opacity: selected ? 1 : 0.92,
          strokeLinecap: 'round',
          strokeLinejoin: 'round',
        }}
      />
    </>
  );
}
