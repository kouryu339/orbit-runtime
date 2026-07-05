import React, { useEffect, useMemo, useRef } from 'react';
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  useReactFlow,
  ReactFlowProvider,
  type Node,
  type Edge,
  MarkerType,
} from '@xyflow/react';
import type { FlowchartData, EditorEvent, FlowchartStepStatus } from '../types';
import { applyDagreLayout } from './layout';
import { StartNode } from './nodes/StartNode';
import { EndNode } from './nodes/EndNode';
import { ActionNode } from './nodes/ActionNode';
import { DecisionNode } from './nodes/DecisionNode';
import { LoopNode } from './nodes/LoopNode';
import { BreakNode } from './nodes/BreakNode';
import { VariableNode } from './nodes/VariableNode';
import { FlowchartEdge } from './edges/FlowchartEdge';
import { KEYFRAMES_CSS } from './nodeShared';

const nodeTypes = {
  start:       StartNode,
  end:         EndNode,
  action:      ActionNode,
  decision:    DecisionNode,
  loop:        LoopNode,
  break:       BreakNode,
  variable:    VariableNode,
  sub_process: ActionNode,
};

const edgeTypes = { flowchart: FlowchartEdge };

interface Props {
  data: FlowchartData | null;
  onEvent?: (event: EditorEvent) => void;
  readonly?: boolean;
  /** AI 当前执行步骤 — 匹配 FlowchartNode.id 或 step_id */
  currentStepId?: string;
  /** 节点状态映射 — key 是 FlowchartNode.id */
  stepStatuses?: Record<string, FlowchartStepStatus>;
}

/* ─── 内部组件（用 useReactFlow，必须在 Provider 内） ───────────── */

function NormalFlowInner({ data, currentStepId, stepStatuses }: Props) {
  const { fitView } = useReactFlow();
  const dataKey = useRef<string>('');

  const { nodes, edges } = useMemo(() => {
    if (!data || data.nodes.length === 0) {
      return { nodes: [] as Node[], edges: [] as Edge[] };
    }

    const layout = applyDagreLayout(data);

    // 合并状态：currentStepId 强制为 running
    const statusOf = (n: { id: string; step_id?: string }): FlowchartStepStatus | undefined => {
      if (currentStepId && (n.id === currentStepId || n.step_id === currentStepId)) {
        return 'running';
      }
      return stepStatuses?.[n.id];
    };

    const rfNodes: Node[] = data.nodes.map((n) => {
      const pos = layout.nodes.get(n.id)!;
      return {
        id:       n.id,
        type:     n.node_type,
        position: { x: pos.x, y: pos.y },
        data: {
          label:     n.label,
          detail:    n.detail,
          outputs:   n.outputs,
          data_from: n.data_from,
          step_id:   n.step_id,
          kind:      n.kind,
          status:    statusOf(n),
        },
        draggable:  false,
        selectable: false,
      };
    });

    const rfEdges: Edge[] = data.edges.map((e, i) => {
      // Decision 节点的 yes/否 边 → 选 sourceHandle 走对应 handle
      const sourceNode = data.nodes.find((n) => n.id === e.source);
      let sourceHandle: string | undefined;
      if (sourceNode?.node_type === 'decision' && e.label) {
        const lbl = String(e.label).toLowerCase();
        if (lbl === '是' || lbl === 'yes') sourceHandle = 'yes';
        else if (lbl === '否' || lbl === 'no') sourceHandle = 'no';
      }

      return {
        id:           `e-${e.source}-${e.target}-${i}`,
        source:       e.source,
        target:       e.target,
        sourceHandle,
        type:         'flowchart',
        label:        e.label,
        markerEnd:    { type: MarkerType.ArrowClosed, color: '#94a3b8', width: 16, height: 16 },
      };
    });

    return { nodes: rfNodes, edges: rfEdges };
  }, [data, currentStepId, stepStatuses]);

  const [rfNodes, setNodes] = useNodesState(nodes);
  const [rfEdges, setEdges] = useEdgesState(edges);

  // data 变化 → 重置节点/边 + 触发 fitView
  useEffect(() => {
    setNodes(nodes);
    setEdges(edges);
    // 数据骨架变了才重 fit；只是 status 变化不重 fit（避免视图乱动）
    const skeleton = data ? `${data.nodes.length}|${data.edges.length}|${data.nodes.map(n => n.id).join(',')}` : '';
    if (skeleton !== dataKey.current) {
      dataKey.current = skeleton;
      // 等 React 把节点提交到 DOM 再 fitView，否则尺寸是 0
      requestAnimationFrame(() => {
        fitView({ padding: 0.18, duration: 300 });
      });
    }
  }, [nodes, edges, data, setNodes, setEdges, fitView]);

  return (
    <ReactFlow
      nodes={rfNodes}
      edges={rfEdges}
      nodeTypes={nodeTypes}
      edgeTypes={edgeTypes}
      fitView
      fitViewOptions={{ padding: 0.18 }}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable={false}
      proOptions={{ hideAttribution: true }}
      style={{ background: '#f8fafc' }}
      onError={() => {}}
    >
      <Background variant={BackgroundVariant.Dots} color="#e2e8f0" gap={20} size={1.2} />
      <Controls
        showInteractive={false}
        style={{
          background: '#ffffff',
          border: '1px solid #e2e8f0',
          borderRadius: 8,
          boxShadow: '0 2px 8px rgba(0,0,0,0.08)',
        }}
      />
      <MiniMap
        pannable
        zoomable
        nodeColor={(n) => {
          switch (n.type) {
            case 'start':    return '#22c55e';
            case 'end':      return '#ef4444';
            case 'decision': return '#f59e0b';
            case 'loop':     return '#8b5cf6';
            case 'variable': return '#14b8a6';
            case 'break':    return '#ef4444';
            default:         return '#3b82f6';
          }
        }}
        maskColor="rgba(248,250,252,0.7)"
        style={{
          background: '#ffffff',
          border: '1px solid #e2e8f0',
          borderRadius: 8,
          boxShadow: '0 2px 8px rgba(0,0,0,0.08)',
        }}
      />
    </ReactFlow>
  );
}

/* ─── 对外组件 — 处理空状态 + Provider ───────────── */

export function NormalFlowEditor(props: Props) {
  const { data } = props;

  // 注入 keyframes（一次性，多个编辑器共存也只注入一次因为 ID 相同）
  useEffect(() => {
    const id = 'fc-normal-keyframes';
    if (document.getElementById(id)) return;
    const style = document.createElement('style');
    style.id = id;
    style.innerHTML = KEYFRAMES_CSS;
    document.head.appendChild(style);
  }, []);

  // 空状态 — 修复死代码 bug：判断必须在 useMemo 之前/外面
  if (!data || data.nodes.length === 0) {
    return (
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          height: '100%',
          background: '#f8fafc',
          flexDirection: 'column',
          gap: 10,
          color: '#94a3b8',
          fontFamily: '"Segoe UI", system-ui, sans-serif',
        }}
      >
        <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.2">
          <rect x="3" y="3" width="7" height="5" rx="1.5" />
          <rect x="14" y="3" width="7" height="5" rx="1.5" />
          <rect x="8" y="16" width="8" height="5" rx="1.5" />
          <path d="M6.5 8v3M17.5 8v3M6.5 11h11M12 11v5" />
        </svg>
        <span style={{ fontSize: 13 }}>暂无流程图数据</span>
      </div>
    );
  }

  return (
    <ReactFlowProvider>
      <NormalFlowInner {...props} />
    </ReactFlowProvider>
  );
}
