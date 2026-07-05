import React, { useMemo, useCallback } from 'react';
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type Connection,
  type IsValidConnection,
  type ReactFlowInstance,
  ConnectionLineType,
} from '@xyflow/react';
import type { BlueprintJson, BlueprintNodeJson, EditorEvent, NodePin } from '../types';
import { getDataTypeColor, EXEC_COLOR } from '../theme';
import { BlueprintNode } from './nodes/BlueprintNode';
import { CommentBoxNode } from './nodes/CommentBoxNode';
import { ExecEdge } from './edges/ExecEdge';
import { DataEdge } from './edges/DataEdge';
import { autoLayoutBlueprintView } from './layout';
import { ContractDrawer } from './ContractDrawer';
import { resolveConnectedPinTypes, withResolvedPinType } from './pinTypeInference';

/* ─── constants ─────────────────────────────────────── */
const BG_COLOR   = '#f7f5ef';
const GRID_COLOR = '#b9cbc6';
const PANEL_BG   = '#fffefa';

const nodeTypes = { blueprint: BlueprintNode, comment: CommentBoxNode };
const edgeTypes = { exec: ExecEdge, data: DataEdge };

function isFinitePosition(node: BlueprintNodeJson) {
  return Number.isFinite(node.position?.x) && Number.isFinite(node.position?.y);
}

function shouldUseFallbackLayout(bp: BlueprintJson) {
  return bp.nodes.length > 0 && bp.nodes.every((node) => {
    const layout = node.properties?.layout as { position_source?: unknown } | undefined;
    return !isFinitePosition(node) || (
      node.position.x === 0 &&
      node.position.y === 0 &&
      layout?.position_source !== 'user' &&
      layout?.position_source !== 'auto'
    );
  });
}

function getVariableTypes(bp: BlueprintJson) {
  const types: Record<string, string> = {};
  for (const variable of bp.variables ?? []) types[variable.name] = variable.data_type || 'Any';
  for (const input of bp.metadata.inputs ?? []) types[input.name] = input.data_type || 'Any';
  return types;
}

function projectResolvedGetVars(bp: BlueprintJson) {
  const variableTypes = getVariableTypes(bp);
  const connectedNames = new Set(
    bp.connections
      .filter((connection) => connection.target_pin === 'Name')
      .map((connection) => connection.target_node)
  );
  const nodes = bp.nodes.map((node) => {
    if (node.node_type !== 'GetVarNode') return node;
    const namePin = node.pins.find((pin) => pin.kind === 'DataInput' && pin.name === 'Name');
    const legacyName = typeof node.properties?.variable_name === 'string'
      ? node.properties.variable_name
      : '';
    const name = connectedNames.has(node.id)
      ? ''
      : typeof namePin?.default_value === 'string'
        ? namePin.default_value
        : legacyName;
    const resolvedType = variableTypes[name] || 'Any';
    return {
      ...node,
      pins: node.pins.map((pin) =>
        pin.kind === 'DataOutput' && pin.name === 'Value'
          ? { ...pin, data_type: resolvedType, resolved_type: resolvedType }
          : pin
      ),
    };
  });
  return { nodes, connections: bp.connections, variableTypes };
}

function pinByName(bp: BlueprintJson | null, nodeId?: string | null, pinName?: string | null): NodePin | null {
  if (!bp || !nodeId || !pinName) return null;
  return bp.nodes.find((node) => node.id === nodeId)?.pins.find((pin) => pin.name === pinName) ?? null;
}

function connectionTypeForPin(pin: NodePin | null) {
  return pin?.kind.startsWith('Exec') ? 'Exec' : 'Data';
}

function canConnect(bp: BlueprintJson | null, conn: Connection) {
  if (!bp || !conn.source || !conn.target || !conn.sourceHandle || !conn.targetHandle) return false;
  if (conn.source === conn.target) return false;
  const sourcePin = pinByName(bp, conn.source, conn.sourceHandle);
  const targetPin = pinByName(bp, conn.target, conn.targetHandle);
  if (!sourcePin || !targetPin) return false;
  if (!sourcePin.kind.endsWith('Output') || !targetPin.kind.endsWith('Input')) return false;
  return connectionTypeForPin(sourcePin) === connectionTypeForPin(targetPin);
}

/* ─── converter ─────────────────────────────────────── */
function blueprintToReactFlow(bp: BlueprintJson, nodeDescriptions?: Record<string, string>, onEvent?: (event: EditorEvent) => void, readonly?: boolean) {
  const projection = projectResolvedGetVars(bp);
  const resolvedPinTypes = resolveConnectedPinTypes(projection.nodes, projection.connections);
  const resolvedNodes = projection.nodes.map((node) => ({
    ...node,
    pins: node.pins.map((pin) =>
      withResolvedPinType(pin, resolvedPinTypes.get(`${node.id}::${pin.name}`))
    ),
  }));
  // Build connected-pin sets per node (used for hollow/filled pin rendering)
  const connectedPins = new Map<string, Set<string>>();
  for (const c of projection.connections) {
    if (!connectedPins.has(c.source_node)) connectedPins.set(c.source_node, new Set());
    if (!connectedPins.has(c.target_node)) connectedPins.set(c.target_node, new Set());
    connectedPins.get(c.source_node)!.add(c.source_pin);
    connectedPins.get(c.target_node)!.add(c.target_pin);
  }

  // Build pin data_type lookup: nodeId+pinName → data_type (for wire colors)
  const pinTypeMap = new Map<string, string>();
  for (const n of resolvedNodes) {
    for (const p of n.pins) {
      pinTypeMap.set(`${n.id}::${p.name}`, p.data_type);
    }
  }

  // Always apply dagre layout — positions from chain_text compilation are all (0,0)
  const layoutMoves = shouldUseFallbackLayout(bp)
    ? autoLayoutBlueprintView(resolvedNodes, projection.connections)
    : [];
  const posMap = new Map(layoutMoves.map(m => [m.node_id, { x: m.x, y: m.y }]));

  const nodes: Node[] = resolvedNodes.map((n) => {
    const hasExec = n.pins.some((p) => p.kind === 'ExecInput' || p.kind === 'ExecOutput');
    const pos = posMap.get(n.id) ?? { x: n.position.x, y: n.position.y };
    return {
      id: n.id,
      type: 'blueprint',
      position: pos,
      data: {
        nodeType:      n.node_type,
        displayName:   n.display_name,
        description:   nodeDescriptions?.[n.node_type],
        pins:          n.pins,
        comment:       n.comment,
        properties:    n.properties,
        size:          n.size,
        isPure:        !hasExec,
        connectedPins: connectedPins.get(n.id) ?? new Set<string>(),
        variableTypes: projection.variableTypes,
        onEvent,
        readonly:      readonly ?? false,
      },
    };
  });

  for (const c of bp.comments ?? []) {
    nodes.push({
      id:       c.id,
      type:     'comment',
      position: { x: c.position.x, y: c.position.y },
      data:     { text: c.text, color: c.color, width: c.size.width, height: c.size.height },
      style:    { width: c.size.width, height: c.size.height },
      zIndex:   -1,
    });
  }

  const edges: Edge[] = projection.connections.map((c) => {
    const isExec    = c.connection_type === 'Exec';
    const dataType  = pinTypeMap.get(`${c.source_node}::${c.source_pin}`) ?? 'Any';
    const wireColor = isExec ? EXEC_COLOR : getDataTypeColor(dataType);
    return {
      id:           c.id || `${c.source_node}:${c.source_pin}-${c.target_node}:${c.target_pin}`,
      source:       c.source_node,
      sourceHandle: c.source_pin,
      target:       c.target_node,
      targetHandle: c.target_pin,
      type:         isExec ? 'exec' : 'data',
      style:        { stroke: wireColor },
      data:         { color: wireColor },
    };
  });

  return { nodes, edges };
}

/* ─── component ─────────────────────────────────────── */
interface Props {
  data: BlueprintJson | null;
  onEvent?: (event: EditorEvent) => void;
  readonly?: boolean;
  nodeDescriptions?: Record<string, string>;
}

export function ProfessionalFlowEditor({ data, onEvent, readonly, nodeDescriptions }: Props) {
  const flowInstance = React.useRef<ReactFlowInstance<Node, Edge> | null>(null);
  const initial = useMemo(() => {
    if (!data) return { nodes: [] as Node[], edges: [] as Edge[] };
    return blueprintToReactFlow(data, nodeDescriptions, onEvent, readonly);
  }, [data, nodeDescriptions, onEvent, readonly]);

  const [nodes, setNodes, onNodesChange] = useNodesState(initial.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initial.edges);

  React.useEffect(() => {
    if (!data) return;
    const rf = blueprintToReactFlow(data, nodeDescriptions, onEvent, readonly);
    setNodes(rf.nodes);
    setEdges(rf.edges);
  }, [data, nodeDescriptions, onEvent, readonly, setNodes, setEdges]);

  const onNodeDragStop = useCallback((_event: MouseEvent | TouchEvent, node: Node) => {
    if (readonly || node.type === 'comment') return;
    const moved = nodes.filter((item) => item.type !== 'comment' && (item.selected || item.id === node.id));
    if (moved.length > 1) {
      onEvent?.({
        type: 'batchmove',
        payload: { moves: moved.map((item) => ({ node_id: item.id, x: item.position.x, y: item.position.y })) },
      });
      return;
    }
    onEvent?.({ type: 'nodemove', payload: { nodeId: node.id, x: node.position.x, y: node.position.y } });
  }, [nodes, onEvent, readonly]);

  const onConnect = useCallback((conn: Connection) => {
    if (readonly || !conn.source || !conn.target || !conn.sourceHandle || !conn.targetHandle) return;
    if (!canConnect(data, conn)) return;
    if (edges.some((edge) =>
      edge.source === conn.source &&
      edge.sourceHandle === conn.sourceHandle &&
      edge.target === conn.target &&
      edge.targetHandle === conn.targetHandle
    )) {
      return;
    }
    onEvent?.({
      type: 'connect',
      payload: {
        sourceNode: conn.source,
        sourcePin:  conn.sourceHandle,
        targetNode: conn.target,
        targetPin:  conn.targetHandle,
      },
    });
  }, [data, edges, onEvent, readonly]);

  const onConnectEnd = useCallback((_event: MouseEvent | TouchEvent, state: {
    fromHandle: { nodeId: string; id?: string | null } | null;
    toHandle: unknown | null;
  }) => {
    if (readonly || state.toHandle || !state.fromHandle?.id) return;
    const connected = edges.some((edge) =>
      (edge.source === state.fromHandle?.nodeId && edge.sourceHandle === state.fromHandle.id) ||
      (edge.target === state.fromHandle?.nodeId && edge.targetHandle === state.fromHandle.id)
    );
    if (connected) {
      onEvent?.({
        type: 'disconnectpin',
        payload: { nodeId: state.fromHandle.nodeId, pinName: state.fromHandle.id },
      });
    }
  }, [edges, onEvent, readonly]);

  const isValidConnection = useCallback<IsValidConnection>((conn) => {
    return canConnect(data, conn as Connection);
  }, [data]);

  const onEdgesDelete = useCallback((deleted: Edge[]) => {
    if (readonly) return;
    for (const e of deleted) {
      if (e.sourceHandle && e.targetHandle) {
        onEvent?.({
          type: 'disconnect',
          payload: {
            sourceNode: e.source,
            sourcePin:  e.sourceHandle,
            targetNode: e.target,
            targetPin:  e.targetHandle,
          },
        });
      }
    }
  }, [onEvent, readonly]);

  const onNodesDelete = useCallback((deleted: Node[]) => {
    if (readonly) return;
    for (const n of deleted) {
      if (n.type === 'comment') {
        onEvent?.({ type: 'commentremove', payload: { commentId: n.id } });
      } else {
        onEvent?.({ type: 'noderemove', payload: { nodeId: n.id } });
      }
    }
  }, [onEvent, readonly]);

  const onNodeClick = useCallback((_event: React.MouseEvent, node: Node) => {
    onEvent?.({ type: 'nodeselect', payload: { nodeId: node.id } });
  }, [onEvent]);

  const onPaneClick = useCallback(() => {
    onEvent?.({ type: 'nodeselect', payload: { nodeId: null } });
  }, [onEvent]);

  const onSelectionChange = useCallback(({ nodes: selectedNodes }: { nodes: Node[] }) => {
    onEvent?.({
      type: 'selectionchange',
      payload: { nodeIds: selectedNodes.filter((node) => node.type !== 'comment').map((node) => node.id) },
    });
  }, [onEvent]);

  const onDragOver = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
  }, []);

  const onDrop = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    if (readonly) return;
    const nodeType = event.dataTransfer.getData('application/workflow-studio-node-type');
    if (!nodeType) return;
    const position = flowInstance.current?.screenToFlowPosition({ x: event.clientX, y: event.clientY });
    onEvent?.({ type: 'nodeadd', payload: { nodeType, x: position?.x ?? 80, y: position?.y ?? 80 } });
  }, [onEvent, readonly]);

  if (!data) {
    return (
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          height: '100%',
          background: BG_COLOR,
          color: '#718781',
          fontSize: 14,
          fontFamily: '"Segoe UI", sans-serif',
          flexDirection: 'column',
          gap: 8,
        }}
      >
        <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
          <rect x="3" y="3" width="7" height="7" rx="1" />
          <rect x="14" y="3" width="7" height="7" rx="1" />
          <rect x="3" y="14" width="7" height="7" rx="1" />
          <path d="M17.5 17.5h.01M14 17.5h3.5M17.5 14v3.5" />
        </svg>
        暂无蓝图数据
      </div>
    );
  }

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%' }}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeDragStop={onNodeDragStop}
        onConnect={onConnect}
        onConnectEnd={onConnectEnd}
        isValidConnection={isValidConnection}
        onEdgesDelete={onEdgesDelete}
        onNodesDelete={onNodesDelete}
        onNodeClick={onNodeClick}
        onPaneClick={onPaneClick}
        onSelectionChange={onSelectionChange}
        onInit={(instance) => { flowInstance.current = instance; }}
        onDragOver={onDragOver}
        onDrop={onDrop}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        fitView
        deleteKeyCode={readonly ? null : 'Delete'}
        nodesDraggable={!readonly}
        nodesConnectable={!readonly}
        elementsSelectable={!readonly}
        proOptions={{ hideAttribution: true }}
        style={{ background: BG_COLOR }}
        defaultEdgeOptions={{ animated: false }}
        connectionLineType={ConnectionLineType.Bezier}
        connectionLineStyle={{ stroke: EXEC_COLOR, strokeWidth: 2.2 }}
        onError={() => {}}
      >
        <Background variant={BackgroundVariant.Dots} color={GRID_COLOR} gap={24} size={1.5} />
        <Controls
          style={{
            background: PANEL_BG,
            border: '1px solid #cbd9d5',
            borderRadius: 8,
            boxShadow: '0 4px 16px rgba(73,104,97,0.15)',
          }}
        />

      {/* MiniMap */}
      <MiniMap
        nodeColor={(n) => {
          if (n.type === 'comment') return '#f59e0b44';
          return '#167d71';
        }}
        maskColor="rgba(247,245,239,0.72)"
        style={{
          background: PANEL_BG,
          border: '1px solid #cbd9d5',
          borderRadius: 8,
          boxShadow: '0 4px 16px rgba(73,104,97,0.15)',
        }}
      />
    </ReactFlow>

    {/* 右侧契约抽屉：INPUT / RETURN / VAR */}
    <ContractDrawer blueprint={data} onEvent={onEvent} readonly={readonly} />

    {/* AI 执行中只读遮罩 */}
    {readonly && (
      <div style={{
        position: 'absolute', inset: 0,
        background: 'rgba(247,245,239,0.72)',
        backdropFilter: 'blur(1px)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        flexDirection: 'column', gap: 10,
        pointerEvents: 'all',
        zIndex: 100,
      }}>
        <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="#167d71" strokeWidth="1.5"
          style={{ animation: 'spin 1.2s linear infinite' }}>
          <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83"/>
        </svg>
        <span style={{ color: '#718781', fontSize: 12, fontFamily: '"Segoe UI", sans-serif' }}>
          AI 正在编辑...
        </span>
        <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
      </div>
    )}
  </div>
  );
}
