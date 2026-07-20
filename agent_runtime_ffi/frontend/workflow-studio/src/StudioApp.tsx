import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Blocks, FileCode2, LayoutDashboard, MessageSquare, RotateCcw, RotateCw, ScrollText, SlidersHorizontal } from 'lucide-react';
import { NodeInspector } from './NodeInspector';
import { WorkflowEditor } from './react-flow/WorkflowEditor';
import { StudioConversation } from './StudioConversation';
import type { BlueprintJson, BlueprintNodeJson, ConnectionJson, EditorEvent, NodePin } from './react-flow/types';
import {
  recordText,
  snapshotRecords,
  type FrontendSnapshotPayload,
  type LedgerRecord,
} from '../../shared/runtimeChat';

type ApiOptions = {
  method?: string;
  body?: unknown;
};

type NodeCapability = {
  name: string;
  display_name?: string;
  description?: string;
  category?: string;
  source?: string;
};

type CatalogGroup = {
  name: string;
  count: number;
  subgroups: { name: string; nodes: NodeCapability[] }[];
};

type WorkflowSummary = {
  id: string;
  name: string;
  description: string;
  kind: 'draft' | 'registered';
  revision: number;
  trusted: boolean;
  production_executable: boolean;
};

type DraftOrigin =
  | { kind: 'scratch'; draft_id: string }
  | { kind: 'workflow_copy'; file_name: string; path?: string; workflow_name?: string }
  | { kind: 'workflow_resource'; workflow_id: string; workflow_name?: string; resource_kind: 'draft' | 'registered'; revision: number };

type WorkflowSnapshotItem = {
  id: string;
  name: string;
  description: string;
  kind: 'draft' | 'registered';
  revision: number;
};

type NormalizeResult = {
  blueprint: BlueprintJson;
  script: string;
  id_remap: Record<string, string>;
  temporary_id_remap: Record<string, string>;
};

type DraftUpdatePayload = {
  schema?: string;
  revision?: number | string;
  script: string;
  blueprint: BlueprintJson;
  origin?: DraftOrigin | null;
  replace_draft?: boolean;
};

type RuntimeEventEnvelope = {
  schema?: string;
  type?: string;
  event_seq?: number;
  conversation_id?: string;
  event_line?: string;
  payload?: FrontendSnapshotPayload | Record<string, unknown>;
};

type SseTraceEntry = {
  key: string;
  eventSeq?: number;
  type: string;
  summary: string;
  raw: unknown;
};

const token = new URLSearchParams(location.search).get('token') ?? '';

function apiUrl(path: string) {
  return `${path}${path.includes('?') ? '&' : '?'}token=${encodeURIComponent(token)}`;
}

async function api<T>(path: string, options: ApiOptions = {}): Promise<T> {
  const response = await fetch(apiUrl(path), {
    method: options.method ?? 'GET',
    headers: { 'Content-Type': 'application/json' },
    body: options.body == null ? undefined : JSON.stringify(options.body),
  });
  const value = await response.json();
  if (!response.ok || value?.error) {
    throw new Error(value?.error ?? `Request failed: ${response.status}`);
  }
  return value as T;
}

function cloneBlueprint<T>(value: T): T {
  return structuredClone(value);
}

function nodeById(blueprint: BlueprintJson, nodeId: string) {
  return blueprint.nodes.find((node) => node.id === nodeId);
}

function pinByName(node: BlueprintNodeJson | undefined, pinName: string) {
  return node?.pins.find((pin) => pin.name === pinName);
}

function connectionTypeForPin(pin: NodePin | undefined) {
  return pin?.kind.startsWith('Exec') ? 'Exec' : 'Data';
}

function exactConnection(conn: ConnectionJson, next: ConnectionJson) {
  return conn.source_node === next.source_node &&
    conn.source_pin === next.source_pin &&
    conn.target_node === next.target_node &&
    conn.target_pin === next.target_pin &&
    conn.connection_type === next.connection_type;
}

function createScratchOrigin(): DraftOrigin {
  return { kind: 'scratch', draft_id: `scratch_${Date.now().toString(36)}` };
}

function buildWorkflowSnapshot(
  workflows: WorkflowSummary[],
  _workflowsDir: string,
  _origin: DraftOrigin,
  _blueprint: BlueprintJson | null
) {
  const items: WorkflowSnapshotItem[] = workflows.map((workflow) => {
    return {
      id: workflow.id,
      name: workflow.name,
      description: workflow.description,
      kind: workflow.kind,
      revision: workflow.revision,
    };
  });
  return {
    schema: 'workflow-studio-workflow-list-snapshot/v2',
    workflows: items,
  };
}

function sseSummary(envelope: RuntimeEventEnvelope): string {
  const records = runtimeEventRecords(envelope);
  const record = records[records.length - 1];
  if (!record) return envelope.type ?? 'event';
  const subtype = record.metadata?.subtype;
  const tool = record.metadata?.tool_name || record.metadata?.extra?.tool_name;
  if (subtype?.startsWith('tool_call_')) return `${subtype}: ${String(tool ?? recordText(record) ?? 'tool')}`;
  return `${record.role ?? 'record'}: ${recordText(record).slice(0, 80)}`;
}

function runtimeEventRecords(envelope: RuntimeEventEnvelope): LedgerRecord[] {
  const payload = envelope.payload as FrontendSnapshotPayload & {
    record?: LedgerRecord;
    records?: LedgerRecord[];
  } | undefined;
  if (!payload) return [];
  if (envelope.type === 'conversation.ledger_delta') {
    if (Array.isArray(payload.records)) return payload.records;
    return payload.record ? [payload.record] : [];
  }
  return snapshotRecords(payload);
}

function setNodeLayoutProperty(node: BlueprintNodeJson, key: string, value: unknown) {
  const layout = typeof node.properties?.layout === 'object' && node.properties.layout && !Array.isArray(node.properties.layout)
    ? node.properties.layout as Record<string, unknown>
    : {};
  node.properties = { ...node.properties, layout: { ...layout, [key]: value } };
}

function nextConnectionId(next: Omit<ConnectionJson, 'id'>) {
  const seed = `${next.source_node}_${next.source_pin}_${next.target_node}_${next.target_pin}`;
  return `conn_${seed}_${Date.now().toString(36)}`;
}

function cloneAsTemporary(node: BlueprintNodeJson, id: string, offset: number) {
  const properties = { ...node.properties };
  delete properties.source_script;
  properties.studio = { temporary: true };
  return {
    ...cloneBlueprint(node),
    id,
    position: { x: node.position.x + offset, y: node.position.y + offset },
    properties,
  };
}

function groupCatalog(nodes: NodeCapability[], query: string): CatalogGroup[] {
  const normalizedQuery = query.trim().toLowerCase();
  const groups = new Map<string, Map<string, NodeCapability[]>>();
  for (const node of nodes) {
    if (normalizedQuery && !`${node.name} ${node.display_name ?? ''} ${node.category ?? ''}`.toLowerCase().includes(normalizedQuery)) {
      continue;
    }
    const category = String(node.category || 'Other');
    const categoryRoot = category.split('/')[0].trim().toLowerCase();
    const identity = `${node.name || ''} ${node.display_name || ''}`.toLowerCase();
    const source = String(node.source || 'local').toLowerCase();
    let group = 'Local tools';
    if (identity.includes('setvar') || identity.includes('set variable')) group = 'Set var';
    else if (categoryRoot === 'control flow') group = 'Flow control';
    else if (source === 'rpc') group = 'RPC tools';
    else if (['math', 'constants', 'array', 'logic', 'string', 'data', 'variable'].includes(categoryRoot)) group = 'Data processing';
    const subgroup = category.split('/').map((part) => part.trim()).filter(Boolean).join(' / ') || 'Other';
    if (!groups.has(group)) groups.set(group, new Map());
    const subgroups = groups.get(group)!;
    if (!subgroups.has(subgroup)) subgroups.set(subgroup, []);
    subgroups.get(subgroup)!.push(node);
  }
  return ['Data processing', 'Flow control', 'Set var', 'Local tools', 'RPC tools']
    .filter((group) => groups.has(group))
    .map((group) => {
      const subgroups = Array.from(groups.get(group)!.entries()).map(([name, groupedNodes]) => ({
        name,
        nodes: groupedNodes,
      }));
      return {
        name: group,
        count: subgroups.reduce((total, subgroup) => total + subgroup.nodes.length, 0),
        subgroups,
      };
    });
}

function applyEditorEvent(blueprint: BlueprintJson, event: EditorEvent) {
  const draft = cloneBlueprint(blueprint);
  const contractPin = (
    name: string,
    kind: 'DataInput' | 'DataOutput',
    dataType: string,
    description = '',
    defaultValue?: unknown,
  ): NodePin => ({
    name,
    kind,
    data_type: dataType,
    description,
    ...(defaultValue !== undefined ? { default_value: defaultValue } : {}),
  });
  const renamePinConnections = (nodeId: string, oldName: string, newName: string) => {
    for (const connection of draft.connections) {
      if (connection.source_node === nodeId && connection.source_pin === oldName) {
        connection.source_pin = newName;
      }
      if (connection.target_node === nodeId && connection.target_pin === oldName) {
        connection.target_pin = newName;
      }
    }
  };
  switch (event.type) {
    case 'nodemove': {
      const node = nodeById(draft, event.payload.nodeId);
      if (!node) return draft;
      node.position = { x: Math.round(event.payload.x), y: Math.round(event.payload.y) };
      setNodeLayoutProperty(node, 'position_source', 'user');
      return draft;
    }
    case 'batchmove':
      for (const move of event.payload.moves) {
        const node = nodeById(draft, move.node_id);
        if (!node) continue;
        node.position = { x: Math.round(move.x), y: Math.round(move.y) };
        setNodeLayoutProperty(node, 'position_source', 'user');
      }
      return draft;
    case 'nodepatch': {
      const node = nodeById(draft, event.payload.nodeId);
      if (!node) return draft;
      if (event.payload.displayName !== undefined) node.display_name = event.payload.displayName;
      if (event.payload.comment !== undefined) node.comment = event.payload.comment;
      return draft;
    }
    case 'nodesizechange': {
      const node = nodeById(draft, event.payload.nodeId);
      if (!node) return draft;
      node.size = {
        width: Math.max(140, Math.round(event.payload.width)),
        height: Math.max(70, Math.round(event.payload.height)),
      };
      setNodeLayoutProperty(node, 'size_source', 'user');
      return draft;
    }
    case 'connect': {
      const sourceNode = nodeById(draft, event.payload.sourceNode);
      const targetNode = nodeById(draft, event.payload.targetNode);
      const sourcePin = pinByName(sourceNode, event.payload.sourcePin);
      const targetPin = pinByName(targetNode, event.payload.targetPin);
      if (!sourceNode || !targetNode || !sourcePin || !targetPin) return draft;
      if (!sourcePin.kind.endsWith('Output') || !targetPin.kind.endsWith('Input')) return draft;
      const connectionType = connectionTypeForPin(sourcePin);
      if (connectionType !== connectionTypeForPin(targetPin)) return draft;
      const next: ConnectionJson = {
        id: nextConnectionId({
          source_node: sourceNode.id,
          source_pin: sourcePin.name,
          target_node: targetNode.id,
          target_pin: targetPin.name,
          connection_type: connectionType,
        }),
        source_node: sourceNode.id,
        source_pin: sourcePin.name,
        target_node: targetNode.id,
        target_pin: targetPin.name,
        connection_type: connectionType,
      };
      draft.connections = draft.connections.filter((conn) => {
        if (exactConnection(conn, next)) return false;
        return !(connectionType === 'Exec' && conn.connection_type === 'Exec' && conn.source_node === next.source_node && conn.source_pin === next.source_pin);
      });
      draft.connections.push(next);
      return draft;
    }
    case 'disconnect':
      draft.connections = draft.connections.filter((conn) =>
        conn.source_node !== event.payload.sourceNode ||
        conn.source_pin !== event.payload.sourcePin ||
        conn.target_node !== event.payload.targetNode ||
        conn.target_pin !== event.payload.targetPin
      );
      return draft;
    case 'disconnectpin':
      draft.connections = draft.connections.filter((conn) =>
        !(conn.source_node === event.payload.nodeId && conn.source_pin === event.payload.pinName) &&
        !(conn.target_node === event.payload.nodeId && conn.target_pin === event.payload.pinName)
      );
      return draft;
    case 'noderemove':
      draft.nodes = draft.nodes.filter((node) => node.id !== event.payload.nodeId);
      draft.connections = draft.connections.filter((conn) =>
        conn.source_node !== event.payload.nodeId && conn.target_node !== event.payload.nodeId
      );
      return draft;
    case 'pindefaultchange': {
      const node = nodeById(draft, event.payload.nodeId);
      const pin = pinByName(node, event.payload.pinName);
      if (pin) pin.default_value = event.payload.value;
      return draft;
    }
    case 'inputdeclare': {
      const start = draft.nodes.find((node) => node.node_type === 'StartNode');
      if (!start || start.pins.some((pin) => pin.name === event.payload.name)) return draft;
      start.pins.push(
        contractPin(event.payload.name, 'DataInput', event.payload.dataType, event.payload.comment),
        contractPin(event.payload.name, 'DataOutput', event.payload.dataType, event.payload.comment),
      );
      draft.metadata.inputs = [
        ...(draft.metadata.inputs ?? []),
        {
          name: event.payload.name,
          data_type: event.payload.dataType,
          description: event.payload.comment ?? '',
        },
      ];
      return draft;
    }
    case 'inputupdate': {
      const start = draft.nodes.find((node) => node.node_type === 'StartNode');
      if (!start) return draft;
      for (const pin of start.pins.filter((item) => item.name === event.payload.name)) {
        if (event.payload.dataType !== undefined) pin.data_type = event.payload.dataType;
        if (event.payload.comment !== undefined) pin.description = event.payload.comment;
        if (pin.kind === 'DataInput' && event.payload.defaultValue !== undefined) {
          pin.default_value = event.payload.defaultValue;
        }
      }
      const metadata = draft.metadata.inputs?.find((item) => item.name === event.payload.name);
      if (metadata) {
        if (event.payload.dataType !== undefined) metadata.data_type = event.payload.dataType;
        if (event.payload.comment !== undefined) metadata.description = event.payload.comment;
        if (event.payload.defaultValue !== undefined) metadata.default_value = event.payload.defaultValue;
      }
      return draft;
    }
    case 'inputrename': {
      const start = draft.nodes.find((node) => node.node_type === 'StartNode');
      if (!start || start.pins.some((pin) => pin.name === event.payload.newName)) return draft;
      for (const pin of start.pins.filter((item) => item.name === event.payload.oldName)) {
        pin.name = event.payload.newName;
      }
      renamePinConnections(start.id, event.payload.oldName, event.payload.newName);
      const metadata = draft.metadata.inputs?.find((item) => item.name === event.payload.oldName);
      if (metadata) metadata.name = event.payload.newName;
      return draft;
    }
    case 'inputremove': {
      const start = draft.nodes.find((node) => node.node_type === 'StartNode');
      if (!start) return draft;
      start.pins = start.pins.filter((pin) => pin.name !== event.payload.name);
      draft.connections = draft.connections.filter((connection) =>
        !(connection.source_node === start.id && connection.source_pin === event.payload.name) &&
        !(connection.target_node === start.id && connection.target_pin === event.payload.name)
      );
      draft.metadata.inputs = (draft.metadata.inputs ?? []).filter((item) => item.name !== event.payload.name);
      return draft;
    }
    case 'returndeclare': {
      const end = draft.nodes.find((node) => node.node_type === 'EndNode');
      if (!end || end.pins.some((pin) => pin.name === event.payload.name)) return draft;
      end.pins.push(contractPin(event.payload.name, 'DataInput', event.payload.dataType, event.payload.comment));
      draft.metadata.outputs = [
        ...(draft.metadata.outputs ?? []),
        {
          name: event.payload.name,
          data_type: event.payload.dataType,
          description: event.payload.comment ?? '',
        },
      ];
      return draft;
    }
    case 'returnupdate': {
      const end = draft.nodes.find((node) => node.node_type === 'EndNode');
      const pin = end?.pins.find((item) => item.kind === 'DataInput' && item.name === event.payload.name);
      if (pin) {
        if (event.payload.dataType !== undefined) pin.data_type = event.payload.dataType;
        if (event.payload.comment !== undefined) pin.description = event.payload.comment;
      }
      const metadata = draft.metadata.outputs?.find((item) => item.name === event.payload.name);
      if (metadata) {
        if (event.payload.dataType !== undefined) metadata.data_type = event.payload.dataType;
        if (event.payload.comment !== undefined) metadata.description = event.payload.comment;
      }
      return draft;
    }
    case 'returnrename': {
      const end = draft.nodes.find((node) => node.node_type === 'EndNode');
      if (!end || end.pins.some((pin) => pin.name === event.payload.newName)) return draft;
      const pin = end.pins.find((item) => item.kind === 'DataInput' && item.name === event.payload.oldName);
      if (pin) pin.name = event.payload.newName;
      renamePinConnections(end.id, event.payload.oldName, event.payload.newName);
      const metadata = draft.metadata.outputs?.find((item) => item.name === event.payload.oldName);
      if (metadata) metadata.name = event.payload.newName;
      return draft;
    }
    case 'returnremove': {
      const end = draft.nodes.find((node) => node.node_type === 'EndNode');
      if (!end) return draft;
      end.pins = end.pins.filter((pin) => pin.name !== event.payload.name);
      draft.connections = draft.connections.filter((connection) =>
        !(connection.source_node === end.id && connection.source_pin === event.payload.name) &&
        !(connection.target_node === end.id && connection.target_pin === event.payload.name)
      );
      draft.metadata.outputs = (draft.metadata.outputs ?? []).filter((item) => item.name !== event.payload.name);
      return draft;
    }
    case 'vardeclare':
      if (!draft.variables.some((item) => item.name === event.payload.name)) {
        draft.variables.push({
          name: event.payload.name,
          data_type: event.payload.dataType,
          default_value: event.payload.defaultValue,
          description: event.payload.comment ?? '',
        });
      }
      return draft;
    case 'varupdate': {
      const variable = draft.variables.find((item) => item.name === event.payload.name);
      if (!variable) return draft;
      if (event.payload.dataType !== undefined) variable.data_type = event.payload.dataType;
      if (event.payload.defaultValue !== undefined) variable.default_value = event.payload.defaultValue;
      if (event.payload.comment !== undefined) variable.description = event.payload.comment;
      return draft;
    }
    case 'varrename': {
      if (draft.variables.some((item) => item.name === event.payload.newName)) return draft;
      const variable = draft.variables.find((item) => item.name === event.payload.oldName);
      if (variable) variable.name = event.payload.newName;
      return draft;
    }
    case 'varremove':
      draft.variables = draft.variables.filter((item) => item.name !== event.payload.name);
      return draft;
    default:
      return draft;
  }
}

export function StudioApp() {
  const [blueprint, setBlueprint] = useState<BlueprintJson | null>(null);
  const [stagingNodes, setStagingNodes] = useState<BlueprintNodeJson[]>([]);
  const [catalog, setCatalog] = useState<NodeCapability[]>([]);
  const [workflows, setWorkflows] = useState<WorkflowSummary[]>([]);
  const [workflowsDir, setWorkflowsDir] = useState('');
  const [draftOrigin, setDraftOriginState] = useState<DraftOrigin>(() => createScratchOrigin());
  const [script, setScript] = useState('workflow demo\nstart -> end');
  const [, setOutput] = useState('Ready.');
  const [sseTrace, setSseTrace] = useState<SseTraceEntry[]>([]);
  const [catalogQuery, setCatalogQuery] = useState('');
  const [centerView, setCenterView] = useState<'blueprint' | 'script'>('blueprint');
  const [rightView, setRightView] = useState<'chat' | 'events' | 'inspector'>('chat');
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [selectedNodeIds, setSelectedNodeIds] = useState<string[]>([]);
  const [editorRevision, setEditorRevision] = useState(0);
  const snapshotTimer = useRef<number | null>(null);
  const lastSnapshot = useRef('');
  const lastPersistedBlueprint = useRef('');
  const dirtyRef = useRef(false);
  const autosaveChain = useRef<Promise<void>>(Promise.resolve());
  const temporarySeq = useRef(0);
  const pasteSeq = useRef(0);
  const clipboardNodes = useRef<BlueprintNodeJson[]>([]);
  const blueprintRef = useRef<BlueprintJson | null>(null);
  const workflowsRef = useRef<WorkflowSummary[]>([]);
  const workflowsDirRef = useRef('');
  const draftOriginRef = useRef<DraftOrigin>(draftOrigin);
  const undoStack = useRef<BlueprintJson[]>([]);
  const redoStack = useRef<BlueprintJson[]>([]);
  const [historyVersion, setHistoryVersion] = useState(0);

  const selectedNode = useMemo(() => {
    if (!selectedNodeId) return null;
    return (blueprint ? nodeById(blueprint, selectedNodeId) : null) ??
      stagingNodes.find((node) => node.id === selectedNodeId) ??
      null;
  }, [blueprint, selectedNodeId, stagingNodes]);
  const connectedSelectedInputNames = useMemo(() => {
    if (!blueprint || !selectedNodeId) return new Set<string>();
    return new Set(
      blueprint.connections
        .filter((connection) => connection.target_node === selectedNodeId)
        .map((connection) => connection.target_pin),
    );
  }, [blueprint, selectedNodeId]);

  const displayBlueprint = useMemo(() => {
    if (!blueprint) return null;
    return { ...blueprint, nodes: [...blueprint.nodes, ...stagingNodes] };
  }, [blueprint, stagingNodes]);
  const canUndo = useMemo(() => undoStack.current.length > 0, [historyVersion]);
  const canRedo = useMemo(() => redoStack.current.length > 0, [historyVersion]);
  const catalogGroups = useMemo(() => groupCatalog(catalog, catalogQuery), [catalog, catalogQuery]);

  const setDraftOrigin = useCallback((next: DraftOrigin) => {
    draftOriginRef.current = next;
    setDraftOriginState(next);
  }, []);

  const setWorkflowState = useCallback((nextWorkflows: WorkflowSummary[], nextWorkflowsDir: string) => {
    workflowsRef.current = nextWorkflows;
    workflowsDirRef.current = nextWorkflowsDir;
    setWorkflows(nextWorkflows);
    setWorkflowsDir(nextWorkflowsDir);
  }, []);

  const decompileDraft = useCallback(async (next: BlueprintJson, reason: string, persist = false) => {
    const workflowSnapshot = buildWorkflowSnapshot(
      workflowsRef.current,
      workflowsDirRef.current,
      draftOriginRef.current,
      next
    );
    const key = JSON.stringify({ blueprint: next, workflowSnapshot });
    if (!persist) {
      if (key === lastSnapshot.current) return;
      const result = await api<{ script: string }>('/api/decompile', {
        method: 'POST',
        body: { blueprint: next, reason, workflow_snapshot: workflowSnapshot },
      });
      lastSnapshot.current = key;
      setScript(result.script);
      return;
    }
    const blueprintKey = JSON.stringify(next);
    if (blueprintKey === lastPersistedBlueprint.current && !dirtyRef.current) return;
    const origin = draftOriginRef.current;
    const result = await api<{
      resource: WorkflowSummary & { script?: string; blueprint?: BlueprintJson };
    }>('/api/save', {
      method: 'POST',
      body: {
        blueprint: next,
        reason,
        workflow_id: origin.kind === 'workflow_resource' ? origin.workflow_id : undefined,
        expected_revision: origin.kind === 'workflow_resource' ? origin.revision : undefined,
      },
    });
    lastSnapshot.current = key;
    lastPersistedBlueprint.current = blueprintKey;
    dirtyRef.current = false;
    setScript(result.resource.script ?? '');
    setDraftOrigin({
      kind: 'workflow_resource',
      workflow_id: result.resource.id,
      workflow_name: result.resource.name,
      resource_kind: result.resource.kind,
      revision: result.resource.revision,
    });
    const saved = await api<{ workflows?: WorkflowSummary[]; workflows_dir?: string }>('/api/workflows');
    setWorkflowState(saved.workflows ?? [], saved.workflows_dir ?? '');
    setOutput(`Synced ${result.resource.name} at revision ${result.resource.revision}`);
  }, [setDraftOrigin, setWorkflowState]);

  const publishWorkflowSnapshot = useCallback(async (nextBlueprint: BlueprintJson | null = blueprintRef.current) => {
    await api('/api/studio-state', {
      method: 'POST',
      body: {
        workflow_snapshot: buildWorkflowSnapshot(
          workflowsRef.current,
          workflowsDirRef.current,
          draftOriginRef.current,
          nextBlueprint
        ),
      },
    });
  }, []);

  const enqueueAutosave = useCallback((next: BlueprintJson, reason: string) => {
    autosaveChain.current = autosaveChain.current
      .catch(() => undefined)
      .then(() => decompileDraft(next, reason, true));
    return autosaveChain.current;
  }, [decompileDraft]);

  const scheduleSnapshot = useCallback((next: BlueprintJson, reason: string, persist = true) => {
    if (snapshotTimer.current != null) window.clearTimeout(snapshotTimer.current);
    snapshotTimer.current = window.setTimeout(() => {
      snapshotTimer.current = null;
      const operation = persist
        ? enqueueAutosave(next, reason)
        : decompileDraft(next, reason, false);
      operation.catch((error) => setOutput(`Workflow sync failed: ${error.message}`));
    }, 350);
  }, [decompileDraft, enqueueAutosave]);

  const commitBlueprint = useCallback((next: BlueprintJson, reason: string, recordHistory = true) => {
    const current = blueprintRef.current;
    if (recordHistory && current && JSON.stringify(current) !== JSON.stringify(next)) {
      undoStack.current.push(cloneBlueprint(current));
      redoStack.current = [];
      setHistoryVersion((version) => version + 1);
    }
    blueprintRef.current = next;
    dirtyRef.current = true;
    setBlueprint(next);
    scheduleSnapshot(next, reason, true);
  }, [scheduleSnapshot]);

  const resetBlueprint = useCallback((next: BlueprintJson, reason: string) => {
    blueprintRef.current = next;
    undoStack.current = [];
    redoStack.current = [];
    setHistoryVersion((version) => version + 1);
    setBlueprint(next);
    dirtyRef.current = false;
    lastPersistedBlueprint.current = JSON.stringify(next);
    scheduleSnapshot(next, reason, false);
  }, [scheduleSnapshot]);

  const undo = useCallback(() => {
    const current = blueprintRef.current;
    const previous = undoStack.current.pop();
    if (!current || !previous) return;
    redoStack.current.push(cloneBlueprint(current));
    commitBlueprint(previous, 'undo', false);
    setHistoryVersion((version) => version + 1);
  }, [commitBlueprint]);

  const redo = useCallback(() => {
    const current = blueprintRef.current;
    const next = redoStack.current.pop();
    if (!current || !next) return;
    undoStack.current.push(cloneBlueprint(current));
    commitBlueprint(next, 'redo', false);
    setHistoryVersion((version) => version + 1);
  }, [commitBlueprint]);

  const copySelection = useCallback(() => {
    const selected = selectedNodeIds.length ? selectedNodeIds : (selectedNodeId ? [selectedNodeId] : []);
    const available = [...(blueprintRef.current?.nodes ?? []), ...stagingNodes];
    clipboardNodes.current = selected
      .map((id) => available.find((node) => node.id === id))
      .filter((node): node is BlueprintNodeJson => Boolean(node))
      .map(cloneBlueprint);
    if (clipboardNodes.current.length) setOutput(`Copied ${clipboardNodes.current.length} node(s).`);
  }, [selectedNodeId, selectedNodeIds, stagingNodes]);

  const pasteSelection = useCallback(() => {
    if (!clipboardNodes.current.length) return;
    const offset = 28 * (++pasteSeq.current);
    const pasted = clipboardNodes.current.map((node) =>
      cloneAsTemporary(node, `tmp_${++temporarySeq.current}`, offset)
    );
    setStagingNodes((nodes) => [...nodes, ...pasted]);
    setSelectedNodeId(pasted[0]?.id ?? null);
    setSelectedNodeIds(pasted.map((node) => node.id));
    setOutput(`Pasted ${pasted.length} temporary node(s). Connect them to the workflow to assign formal IDs.`);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.matches('input, textarea, [contenteditable="true"]')) return;
      if (!(event.ctrlKey || event.metaKey)) return;
      const key = event.key.toLowerCase();
      if (key === 'z' && event.shiftKey) {
        event.preventDefault();
        redo();
      } else if (key === 'z') {
        event.preventDefault();
        undo();
      } else if (key === 'y') {
        event.preventDefault();
        redo();
      } else if (key === 'c') {
        event.preventDefault();
        copySelection();
      } else if (key === 'v') {
        event.preventDefault();
        pasteSelection();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [copySelection, pasteSelection, redo, undo]);

  const normalizeCandidate = useCallback(async (candidate: BlueprintJson, temporaryIds: string[] = []) => {
    const result = await api<NormalizeResult>('/api/normalize', {
      method: 'POST',
      body: { blueprint: candidate, temporary_ids: temporaryIds },
    });
    commitBlueprint(result.blueprint, 'normalize', true);
    setScript(result.script);
    lastSnapshot.current = JSON.stringify(result.blueprint);
    if (temporaryIds.length) {
      setStagingNodes((nodes) => nodes.filter((node) => !temporaryIds.includes(node.id)));
    }
    setSelectedNodeId((current) => result.temporary_id_remap[current ?? ''] ?? result.id_remap[current ?? ''] ?? current);
  }, [commitBlueprint]);

  const stageNode = useCallback(async (nodeType: string, x: number, y: number) => {
    const temporaryId = `tmp_${++temporarySeq.current}`;
    const result = await api<{ node: BlueprintNodeJson }>('/api/nodes/instantiate', {
      method: 'POST',
      body: { node_type: nodeType, temporary_id: temporaryId, position: { x, y } },
    });
    setStagingNodes((nodes) => [...nodes, result.node]);
    setSelectedNodeId(temporaryId);
  }, []);

  const onEvent = useCallback(async (event: EditorEvent) => {
    if (event.type === 'nodeselect') {
      setSelectedNodeId(event.payload.nodeId);
      if (event.payload.nodeId) setRightView('inspector');
      return;
    }
    if (event.type === 'selectionchange') {
      setSelectedNodeIds(event.payload.nodeIds);
      return;
    }
    if (event.type === 'nodeadd') {
      await stageNode(event.payload.nodeType, event.payload.x, event.payload.y);
      return;
    }
    const stagingId = event.type === 'nodemove' || event.type === 'noderemove' ||
      event.type === 'nodepatch' || event.type === 'nodesizechange' || event.type === 'pindefaultchange'
      ? event.payload.nodeId
      : null;
    if (stagingId && stagingNodes.some((node) => node.id === stagingId)) {
      if (event.type === 'noderemove') {
        setStagingNodes((nodes) => nodes.filter((node) => node.id !== stagingId));
      } else {
        setStagingNodes((nodes) => applyEditorEvent({
          version: blueprint?.version ?? '1',
          metadata: blueprint?.metadata ?? {
            name: 'temporary', created: '', modified: '', description: '', author: '', tags: [],
            visibility: 'private', inputs: [], outputs: [],
          },
          nodes,
          connections: [],
          variables: [],
          comments: [],
        }, event).nodes);
      }
      return;
    }
    if (!blueprint) return;
    if (event.type === 'batchmove') {
      const formalMoves = event.payload.moves.filter((move) => blueprint.nodes.some((node) => node.id === move.node_id));
      const stagingMoves = event.payload.moves.filter((move) => stagingNodes.some((node) => node.id === move.node_id));
      if (formalMoves.length) commitBlueprint(applyEditorEvent(blueprint, { type: 'batchmove', payload: { moves: formalMoves } }), 'batchmove');
      if (stagingMoves.length) {
        setStagingNodes((nodes) => nodes.map((node) => {
          const move = stagingMoves.find((item) => item.node_id === node.id);
          return move ? { ...node, position: { x: Math.round(move.x), y: Math.round(move.y) } } : node;
        }));
      }
      return;
    }
    if (event.type === 'nodemove' || event.type === 'pindefaultchange' ||
      event.type === 'nodepatch' || event.type === 'nodesizechange' ||
      event.type === 'inputdeclare' || event.type === 'inputupdate' ||
      event.type === 'inputrename' || event.type === 'inputremove' ||
      event.type === 'returndeclare' || event.type === 'returnupdate' ||
      event.type === 'returnrename' || event.type === 'returnremove' ||
      event.type === 'vardeclare' || event.type === 'varupdate' ||
      event.type === 'varrename' || event.type === 'varremove') {
      commitBlueprint(applyEditorEvent(blueprint, event), event.type);
      return;
    }
    if (event.type === 'connect' || event.type === 'disconnect' ||
      event.type === 'disconnectpin' || event.type === 'noderemove') {
      const temporaryIds = event.type === 'connect'
        ? [event.payload.sourceNode, event.payload.targetNode].filter((id) => stagingNodes.some((node) => node.id === id))
        : [];
      if (event.type === 'connect' && temporaryIds.length === 2) {
        setOutput('Temporary nodes become formal after they connect to the workflow. Connect one of these nodes to a formal node first.');
        return;
      }
      const candidate = {
        ...blueprint,
        nodes: [...blueprint.nodes, ...stagingNodes.filter((node) => temporaryIds.includes(node.id))],
      };
      try {
        await normalizeCandidate(applyEditorEvent(candidate, event), temporaryIds);
      } catch (error) {
        setOutput(`Normalize failed: ${(error as Error).message}`);
      }
    }
  }, [blueprint, commitBlueprint, normalizeCandidate, stageNode, stagingNodes]);

  const appendSseTrace = useCallback((event: RuntimeEventEnvelope) => {
    const key = `${event.event_seq ?? 'event'}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setSseTrace((items) => [
      ...items,
      {
        key,
        eventSeq: event.event_seq,
        type: event.type ?? 'unknown',
        summary: sseSummary(event),
        raw: event,
      },
    ].slice(-120));
  }, []);

  const autoLayout = useCallback(async () => {
    if (!blueprint) return;
    const result = await api<{ blueprint: BlueprintJson; algorithm: string }>('/api/layout', { method: 'POST', body: { blueprint, mode: 'all' } });
    commitBlueprint(result.blueprint, 'layout');
    setOutput(JSON.stringify({ layout: result.algorithm, nodes: result.blueprint.nodes.length }, null, 2));
  }, [blueprint, commitBlueprint]);

  const applyDraftUpdate = useCallback((result: DraftUpdatePayload) => {
    setStagingNodes([]);
    setSelectedNodeId(null);
    setSelectedNodeIds([]);
    setEditorRevision((revision) => revision + 1);
    if (result.origin) setDraftOrigin(result.origin);
    setScript(result.script);
    const reason = result.replace_draft ? 'agent_open_draft' : 'agent_draft';
    if (result.replace_draft) {
      resetBlueprint(result.blueprint, reason);
    } else if (blueprintRef.current) {
      commitBlueprint(result.blueprint, reason, true);
    } else {
      resetBlueprint(result.blueprint, reason);
    }
    decompileDraft(result.blueprint, reason).catch((error) => setOutput(`Decompile failed: ${error.message}`));
    setCenterView('blueprint');
  }, [commitBlueprint, decompileDraft, resetBlueprint, setDraftOrigin]);

  useEffect(() => {
    Promise.all([
      api<{
        editor_conversation_id?: string;
        node_capabilities?: NodeCapability[];
        workflows_dir?: string;
      }>('/api/context'),
      api<{ workflows?: WorkflowSummary[]; workflows_dir?: string }>('/api/workflows'),
    ]).then(([context, saved]) => {
      setCatalog(context.node_capabilities ?? []);
      setWorkflowState(saved.workflows ?? [], saved.workflows_dir ?? context.workflows_dir ?? '');
      publishWorkflowSnapshot(null).catch((error) => setOutput(`Workflow snapshot failed: ${error.message}`));
    }).catch((error) => setOutput(`Load context failed: ${error.message}`));
  }, [publishWorkflowSnapshot, setWorkflowState]);

  const refreshSelectedWorkflow = useCallback(async (
    expectedWorkflowId?: string,
    operation?: string,
  ) => {
    const saved = await api<{
      workflows?: WorkflowSummary[];
      workflows_dir?: string;
      selection?: { workflow_id: string; revision: number } | null;
    }>('/api/workflows');
    setWorkflowState(saved.workflows ?? [], saved.workflows_dir ?? '');
    if (!saved.selection || operation === 'deleted') return;
    if (expectedWorkflowId && saved.selection.workflow_id !== expectedWorkflowId) return;
    if (dirtyRef.current) {
      setOutput('Workflow changed externally while local edits are pending; syncing will use the local expected revision.');
      return;
    }
    const detail = await api<{
      resource: WorkflowSummary & { script?: string; blueprint?: BlueprintJson };
      blueprint?: BlueprintJson;
    }>('/api/workflows/' + encodeURIComponent(saved.selection.workflow_id));
    const nextBlueprint = detail.blueprint ?? detail.resource.blueprint;
    if (!nextBlueprint) return;
    applyDraftUpdate({
      script: detail.resource.script ?? '',
      blueprint: nextBlueprint,
      origin: {
        kind: 'workflow_resource',
        workflow_id: detail.resource.id,
        workflow_name: detail.resource.name,
        resource_kind: detail.resource.kind,
        revision: detail.resource.revision,
      },
      replace_draft: true,
    });
    await publishWorkflowSnapshot(nextBlueprint);
  }, [applyDraftUpdate, publishWorkflowSnapshot, setWorkflowState]);

  useEffect(() => {
    const source = new EventSource(apiUrl('/events'));
    source.onmessage = (message) => {
      if (!message.data) return;
      try {
        const event = JSON.parse(message.data) as RuntimeEventEnvelope;
        appendSseTrace(event);
        if (event.event_line === 'workflow') {
          const payload = event.payload as { workflow_id?: string; operation?: string } | undefined;
          void refreshSelectedWorkflow(payload?.workflow_id, payload?.operation)
            .catch((error) => setOutput(`Workflow refresh failed: ${error.message}`));
          return;
        }
        const selectionChanged = runtimeEventRecords(event).some((record) =>
          record.metadata?.subtype === 'tool_call_finished' &&
          record.metadata?.tool_name === 'openWorkflowDraft'
        );
        if (selectionChanged) {
          void refreshSelectedWorkflow()
            .catch((error) => setOutput(`Workflow selection refresh failed: ${error.message}`));
        }
      } catch (error) {
        console.warn('Failed to parse Workflow Studio event', error);
      }
    };
    source.onerror = () => setOutput('Event stream reconnecting...');
    return () => source.close();
  }, [appendSseTrace, refreshSelectedWorkflow]);

  const flushAutosave = useCallback(async (reason: string) => {
    const current = blueprintRef.current;
    if (!current) return;
    if (snapshotTimer.current != null) {
      window.clearTimeout(snapshotTimer.current);
      snapshotTimer.current = null;
    }
    await enqueueAutosave(current, reason);
    await publishWorkflowSnapshot(current);
  }, [enqueueAutosave, publishWorkflowSnapshot]);

  const saveWorkflow = useCallback(async () => {
    await flushAutosave('manual_save');
  }, [flushAutosave]);

  const testWorkflow = useCallback(async () => {
    if (!blueprint) return;
    await flushAutosave('before_test');
    const result = await api<unknown>('/api/run', { method: 'POST', body: { inputs: {}, trace: true } });
    await api('/api/chat', {
      method: 'POST',
      body: {
        message: `Analyze the latest workflow test result for the user. Summarize the outcome, identify failures or suspicious behavior, and suggest the next useful edit if needed.\n\n${JSON.stringify(result, null, 2)}`,
      },
    });
  }, [blueprint, flushAutosave]);

  return (
    <div className="studio-shell">
      <header className="studio-topbar">
        <div>
          <h1>Workflow Studio</h1>
          <span>
            {blueprint ? `${blueprint.nodes.length} nodes / ${blueprint.connections.length} wires` : 'no draft loaded'}
            {' / '}
            {draftOrigin.kind === 'workflow_copy'
              ? `copy of ${draftOrigin.workflow_name || draftOrigin.file_name}`
              : draftOrigin.kind === 'workflow_resource'
                ? `${draftOrigin.workflow_name || draftOrigin.workflow_id} / ${draftOrigin.resource_kind} r${draftOrigin.revision}`
                : 'scratch draft'}
          </span>
        </div>
        <div className="studio-actions">
          <div className="studio-switcher">
            <button className={centerView === 'blueprint' ? 'active' : ''} title="Blueprint view" onClick={() => setCenterView('blueprint')}>
              <LayoutDashboard size={15} />
              Blueprint
            </button>
            <button className={centerView === 'script' ? 'active' : ''} title="Script view" onClick={() => setCenterView('script')}>
              <FileCode2 size={15} />
              Script
            </button>
          </div>
          <button className="icon-button" title="Undo" onClick={undo} disabled={!canUndo}>
            <RotateCcw size={15} />
          </button>
          <button className="icon-button" title="Redo" onClick={redo} disabled={!canRedo}>
            <RotateCw size={15} />
          </button>
          <button onClick={autoLayout} disabled={!blueprint}>Auto layout</button>
          <button onClick={saveWorkflow} disabled={!blueprint}>Save</button>
          <button onClick={testWorkflow} disabled={!blueprint}>Test</button>
        </div>
      </header>
      <main className="studio-main">
        <aside className="studio-library">
          <div className="panel-title">
            <Blocks size={14} />
            Nodes
          </div>
              <input
                className="studio-library-search"
                value={catalogQuery}
                onChange={(event) => setCatalogQuery(event.target.value)}
                placeholder="Filter nodes"
              />
              <div className="studio-catalog-tree">
                {catalogGroups.map((group) => (
                  <details className="catalog-group" key={group.name} open>
                    <summary>
                      <b>{group.name}</b>
                      <span>{group.count}</span>
                    </summary>
                    {group.subgroups.map((subgroup) => (
                      <details className="catalog-subgroup" key={`${group.name}:${subgroup.name}`} open={Boolean(catalogQuery)}>
                        <summary>
                          <b>{subgroup.name}</b>
                          <span>{subgroup.nodes.length}</span>
                        </summary>
                        <div className="studio-library-list">
                          {subgroup.nodes.map((node) => (
                            <button
                              key={`${node.source ?? 'local'}:${node.name}`}
                              title={node.description}
                              draggable
                              onDragStart={(event) => {
                                event.dataTransfer.setData('application/workflow-studio-node-type', node.name);
                                event.dataTransfer.effectAllowed = 'copy';
                              }}
                            >
                              <b>{node.display_name || node.name}</b>
                              <span>{node.source || 'local'} · {node.category || 'node'}</span>
                            </button>
                          ))}
                        </div>
                      </details>
                    ))}
                  </details>
                ))}
                {!catalogGroups.length && <small>No matching nodes</small>}
              </div>
        </aside>
        <section className="studio-canvas">
          {centerView === 'blueprint' ? (
            <WorkflowEditor
              key={`blueprint-${editorRevision}`}
              mode="professional"
              blueprintData={displayBlueprint ?? undefined}
              onEvent={onEvent}
            />
          ) : (
            <textarea
              className="studio-script-editor"
              value={script}
              readOnly
              spellCheck={false}
              title="Read-only script view generated from the current BlueprintJson draft. Ask the editor agent to rewrite script drafts."
            />
          )}
        </section>
        <aside className="studio-agent">
          <div className="panel-tabs">
            <button className={rightView === 'chat' ? 'active' : ''} onClick={() => setRightView('chat')}>
              <MessageSquare size={14} />
              Chat
            </button>
            <button className={rightView === 'events' ? 'active' : ''} onClick={() => setRightView('events')}>
              <ScrollText size={14} />
              Events
            </button>
            <button className={rightView === 'inspector' ? 'active' : ''} onClick={() => setRightView('inspector')}>
              <SlidersHorizontal size={14} />
              Inspector
            </button>
          </div>
          {rightView === 'chat' ? (
            <div className="studio-chat">
              <StudioConversation
                token={token}
                locale="en-US"
                theme="green"
                beforeSend={async () => {
                  await flushAutosave('before_chat');
                }}
                onError={(message) => setOutput(`Conversation error: ${message}`)}
              />
            </div>
          ) : rightView === 'events' ? (
            <div className="studio-events">
              {sseTrace.length ? sseTrace.slice().reverse().map((entry) => (
                <details key={entry.key} className="studio-event-row">
                  <summary>
                    <span>{entry.eventSeq ?? '-'}</span>
                    <b>{entry.type}</b>
                    <em>{entry.summary}</em>
                  </summary>
                  <pre>{JSON.stringify(entry.raw, null, 2)}</pre>
                </details>
              )) : (
                <p>No SSE events received yet.</p>
              )}
            </div>
          ) : (
            <div className="studio-inspector">
              <NodeInspector
                node={selectedNode}
                connectedInputNames={connectedSelectedInputNames}
                onEvent={onEvent}
              />
            </div>
          )}
        </aside>
      </main>
    </div>
  );
}
