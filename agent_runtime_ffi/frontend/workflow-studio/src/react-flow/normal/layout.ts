import dagre from 'dagre';
import type { FlowchartData, FlowchartNode } from '../types';

export interface LayoutNode {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface LayoutResult {
  nodes: Map<string, LayoutNode>;
}

// ── Node sizes — 与新版（v2）节点组件实际渲染对齐 ─────────────────────────
//
// 所有节点都加了图标 + 可能的 outputs/data_from 行，整体变高。
// 这里给的是"典型尺寸"，dagre 用它做布局间距，实际渲染会自适应。
//
// ActionNode / LoopNode: 含图标 + 标题 + 可选 outputs/data_from 两行 → 100
// DecisionNode: 卡片化（不再菱形），高度同 Action 但少一行 → 60
// StartNode / EndNode: 图标 + 标题 + 可选 detail 行 → 64
// VariableNode: 单行小卡片 → 36
// BreakNode: ≈ variable
const NODE_SIZES: Record<string, { w: number; h: number }> = {
  start:       { w: 200, h: 64 },
  end:         { w: 200, h: 64 },
  action:      { w: 260, h: 100 },
  decision:    { w: 220, h: 60 },
  loop:        { w: 260, h: 100 },
  break:       { w: 140, h: 36 },
  variable:    { w: 200, h: 36 },
  sub_process: { w: 260, h: 100 },
};

function getNodeSize(node: FlowchartNode): { w: number; h: number } {
  return NODE_SIZES[node.node_type] ?? { w: 200, h: 52 };
}

export function applyDagreLayout(fc: FlowchartData): LayoutResult {
  const g = new dagre.graphlib.Graph();
  g.setGraph({
    rankdir: 'TB',
    nodesep: 80,   // horizontal gap between sibling nodes in same rank
    ranksep: 110,  // vertical gap between ranks
    marginx: 40,
    marginy: 40,
  });
  g.setDefaultEdgeLabel(() => ({}));

  for (const node of fc.nodes) {
    const { w, h } = getNodeSize(node);
    g.setNode(node.id, { width: w, height: h });
  }

  for (const edge of fc.edges) {
    // Decision "no" branch typically goes to a node at the same depth or higher.
    // Give it minlen=0 so dagre can place the target in the same or nearby rank,
    // keeping the "no" path horizontal rather than forcing it a full rank down.
    const isDecisionNoBranch =
      fc.nodes.find((n) => n.id === edge.source)?.node_type === 'decision' &&
      edge.label != null &&
      String(edge.label).toLowerCase() !== 'yes' &&
      String(edge.label).toLowerCase() !== '是';

    g.setEdge(edge.source, edge.target, {
      weight:  isDecisionNoBranch ? 1 : 2,  // main flow edges pull harder
      minlen:  isDecisionNoBranch ? 0 : 1,  // no-branch can share rank
    });
  }

  dagre.layout(g);

  const nodes = new Map<string, LayoutNode>();
  for (const node of fc.nodes) {
    const pos = g.node(node.id);
    const { w, h } = getNodeSize(node);
    nodes.set(node.id, {
      id:     node.id,
      x:      Math.round(pos.x - w / 2),
      y:      Math.round(pos.y - h / 2),
      width:  w,
      height: h,
    });
  }

  return { nodes };
}
