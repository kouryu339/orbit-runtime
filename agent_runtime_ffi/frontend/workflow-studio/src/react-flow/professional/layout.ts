import dagre from 'dagre';
import type { BlueprintJson, BlueprintNodeJson, ConnectionJson, NodeMoveItem } from '../types';

/* ─── node size estimation ───────────────────────────── */
// Keep these in sync with BlueprintNode.tsx constants
const HEADER_H   = 32;   // matches HEADER_HEIGHT in BlueprintNode.tsx
const PIN_ROW_H  = 26;   // matches PIN_ROW_H in BlueprintNode.tsx
const PIN_AREA_PAD = 4;  // padding: '2px 0' → 2+2=4px total (BlueprintNode.tsx line 436)
const COMMENT_H  = 36;   // padding:'4px 10px 6px' (10px) + ~1 line at 14px + border ≈ 36px
const CHAR_W     = 6.5;  // matches BlueprintNode.tsx CHAR_W
const PIN_OVERHEAD = 26; // matches BlueprintNode.tsx PIN_OVERHEAD
const NODE_MIN_W = 180;  // matches BlueprintNode.tsx NODE_MIN_W
const NODE_MAX_W = 360;  // matches BlueprintNode.tsx NODE_MAX_W
export const VARIABLE_SOURCE_NODE_WIDTH = 150;
export const VARIABLE_SOURCE_NODE_HEIGHT = 58;

/** Mirror of estimateNodeWidth() in BlueprintNode.tsx */
function estimateNodeWidth(node: BlueprintNodeJson): number {
  if (node.node_type === 'VariableSourceNode') return VARIABLE_SOURCE_NODE_WIDTH;
  const inputPins  = node.pins.filter((p) => p.kind === 'ExecInput'  || p.kind === 'DataInput');
  const outputPins = node.pins.filter((p) => p.kind === 'ExecOutput' || p.kind === 'DataOutput');
  const maxInLen   = inputPins.reduce((m, p) => Math.max(m, p.name.length), 0);
  const maxOutLen  = outputPins.reduce((m, p) => Math.max(m, p.name.length), 0);
  const w = (maxInLen + maxOutLen) * CHAR_W + PIN_OVERHEAD * 2 + 16;
  return Math.min(NODE_MAX_W, Math.max(NODE_MIN_W, Math.round(w)));
}

function estimateNodeHeight(node: BlueprintNodeJson): number {
  if (node.node_type === 'VariableSourceNode') return VARIABLE_SOURCE_NODE_HEIGHT;
  const inputs  = node.pins.filter((p) => p.kind === 'ExecInput'  || p.kind === 'DataInput').length;
  const outputs = node.pins.filter((p) => p.kind === 'ExecOutput' || p.kind === 'DataOutput').length;
  const maxPins = Math.max(inputs, outputs, 1);
  // description row: only when description contains '{{' template syntax
  // We don't have description in BlueprintNodeJson, so skip it (conservative)
  return HEADER_H + PIN_AREA_PAD + maxPins * PIN_ROW_H + (node.comment ? COMMENT_H : 0);
}

/* ─── main layout function ───────────────────────────── */

/**
 * Compute auto-layout positions for all blueprint nodes using Dagre (LR direction).
 * Returns an array of NodeMoveItem ready to pass to `batchMoveNodes`.
 */
export function autoLayoutBlueprint(bp: BlueprintJson): NodeMoveItem[] {
  if (!bp.nodes.length) return [];

  const g = new dagre.graphlib.Graph({ multigraph: true });
  g.setGraph({
    rankdir: 'LR',   // left-to-right, like UE5 blueprints
    nodesep: 60,     // vertical gap between nodes in the same rank
    ranksep: 140,    // horizontal gap between ranks
    marginx: 60,
    marginy: 60,
  });
  g.setDefaultEdgeLabel(() => ({}));

  // Add nodes with estimated sizes (width is dynamic, matching BlueprintNode.tsx)
  for (const node of bp.nodes) {
    g.setNode(node.id, {
      width:  estimateNodeWidth(node),
      height: estimateNodeHeight(node),
    });
  }

  // Add edges — prefer Exec connections for rank ordering
  for (const conn of bp.connections) {
    const isExec = conn.connection_type === 'Exec';
    g.setEdge(conn.source_node, conn.target_node, {
      weight: isExec ? 3 : 1,  // exec edges have higher weight → stronger rank pull
    }, conn.id || `${conn.source_node}-${conn.target_node}`);
  }

  dagre.layout(g);

  return bp.nodes.map((node) => {
    const pos = g.node(node.id);
    const w = estimateNodeWidth(node);
    const h = estimateNodeHeight(node);
    return {
      node_id: node.id,
      x: Math.round(pos.x - w / 2),
      y: Math.round(pos.y - h / 2),
    };
  });
}

export function autoLayoutBlueprintView(nodes: BlueprintNodeJson[], connections: ConnectionJson[]): NodeMoveItem[] {
  if (!nodes.length) return [];

  const g = new dagre.graphlib.Graph({ multigraph: true });
  g.setGraph({
    rankdir: 'LR',
    nodesep: 60,
    ranksep: 140,
    marginx: 60,
    marginy: 60,
  });
  g.setDefaultEdgeLabel(() => ({}));

  for (const node of nodes) {
    g.setNode(node.id, {
      width: estimateNodeWidth(node),
      height: estimateNodeHeight(node),
    });
  }

  for (const conn of connections) {
    g.setEdge(conn.source_node, conn.target_node, {
      weight: conn.connection_type === 'Exec' ? 3 : 1,
    }, conn.id || `${conn.source_node}-${conn.target_node}`);
  }

  dagre.layout(g);

  return nodes.map((node) => {
    const pos = g.node(node.id);
    const w = estimateNodeWidth(node);
    const h = estimateNodeHeight(node);
    return {
      node_id: node.id,
      x: Math.round(pos.x - w / 2),
      y: Math.round(pos.y - h / 2),
    };
  });
}
