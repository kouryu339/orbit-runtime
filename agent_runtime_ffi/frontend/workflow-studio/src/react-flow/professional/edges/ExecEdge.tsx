import React from 'react';
import { BaseEdge, getBezierPath, type EdgeProps } from '@xyflow/react';
import { EXEC_COLOR } from '../../theme';

/**
 * Blueprint-style execution wire.
 */
export function ExecEdge(props: EdgeProps) {
  const { sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, markerEnd, selected } = props;
  const [edgePath] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
    curvature: 0.42,
  });

  return (
    <>
      <BaseEdge
        path={edgePath}
        style={{
          stroke: 'rgba(255, 254, 250, 0.9)',
          strokeWidth: selected ? 6 : 4.8,
        }}
      />
      <BaseEdge
        path={edgePath}
        markerEnd={markerEnd}
        style={{
          stroke: EXEC_COLOR,
          strokeWidth: selected ? 3.2 : 2.4,
          opacity: selected ? 1 : 0.94,
          strokeLinecap: 'round',
          strokeLinejoin: 'round',
        }}
      />
    </>
  );
}
