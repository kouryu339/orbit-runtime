// ─── Blueprint JSON types (mirrors Rust structs) ───

export interface NodePosition {
  x: number;
  y: number;
}

export interface CommentSize {
  width: number;
  height: number;
}

export interface NodeSize {
  width: number;
  height: number;
}

export interface SplitField {
  field_name: string;
  pin_name: string;
  data_type: string;
}

export interface SplitConfig {
  is_split: boolean;
  split_fields: SplitField[];
}

export interface NodePin {
  name: string;
  kind: string; // "ExecInput" | "ExecOutput" | "DataInput" | "DataOutput"
  data_type: string;
  description: string;
  default_value?: unknown;
  resolved_type?: unknown;
  split_config?: SplitConfig;
}

export interface BlueprintNodeJson {
  id: string;
  node_type: string;
  position: NodePosition;
  size?: NodeSize;
  pins: NodePin[];
  properties: Record<string, unknown>;
  display_name?: string;
  comment?: string;
}

export interface ConnectionJson {
  id: string;
  source_node: string;
  source_pin: string;
  target_node: string;
  target_pin: string;
  connection_type: string;
}

export interface CommentBox {
  id: string;
  text: string;
  position: NodePosition;
  size: CommentSize;
  color?: string;
}

export interface PinMetadata {
  name: string;
  data_type: string;
  description: string;
  default_value?: unknown;
}

export interface BlueprintMetadata {
  name: string;
  created: string;
  modified: string;
  description: string;
  author: string;
  tags: string[];
  visibility: string;
  inputs: PinMetadata[];
  outputs: PinMetadata[];
}

export interface BlueprintVariable {
  name: string;
  data_type: string;
  default_value?: unknown;
  description: string;
}

export interface BlueprintJson {
  version: string;
  metadata: BlueprintMetadata;
  nodes: BlueprintNodeJson[];
  connections: ConnectionJson[];
  variables: BlueprintVariable[];
  comments: CommentBox[];
}

// ─── Flowchart types (normal mode) ───

export type FlowchartNodeType =
  | 'start' | 'end' | 'action' | 'decision'
  | 'loop' | 'break' | 'variable' | 'sub_process';

/** AI 执行态：让普通用户看到"工作流跑到哪一步了" */
export type FlowchartStepStatus = 'pending' | 'running' | 'completed' | 'error';

export interface FlowchartNode {
  id: string;
  node_type: FlowchartNodeType;
  label: string;
  detail?: string;
  depth: number;
  step_id?: string;
  outputs?: string[];
  data_from?: string[];
  /** 可选：节点对应的注册表节点类型（如 OpenBrowserNode），用于查图标 */
  kind?: string;
}

export interface FlowchartEdge {
  source: string;
  target: string;
  label?: string;
}

export interface FlowchartData {
  nodes: FlowchartNode[];
  edges: FlowchartEdge[];
}

// ─── Node catalog types ───

export interface PinInfo {
  name: string;
  kind: string;
  data_type: string;
  description: string;
  default_value?: unknown;
}

export interface NodeTypeInfo {
  node_type: string;
  display_name: string;
  description: string;
  category: string;
  pins: PinInfo[];
  permissions: number;
}

export interface CategoryInfo {
  name: string;
  description: string;
  node_count: number;
}

export interface NodeCatalogResponse {
  categories: CategoryInfo[];
  nodes: NodeTypeInfo[];
}

// ─── Draft operation types ───

export interface DraftOpOutput {
  success: boolean;
  message: string;
  id?: string;
}

export interface WorkflowDraft {
  blueprint: BlueprintJson;
  chain_text: string;
}

export interface DraftGetOutput {
  draft: WorkflowDraft | null;
}

export interface NodeMoveItem {
  node_id: string;
  x: number;
  y: number;
}

// ─── Editor event types ───

export type EditorEvent =
  | { type: 'nodeadd'; payload: { nodeType: string; x: number; y: number } }
  | { type: 'nodemove'; payload: { nodeId: string; x: number; y: number } }
  | { type: 'batchmove'; payload: { moves: NodeMoveItem[] } }
  | { type: 'noderemove'; payload: { nodeId: string } }
  | { type: 'nodepatch'; payload: { nodeId: string; displayName?: string; comment?: string } }
  | { type: 'nodesizechange'; payload: { nodeId: string; width: number; height: number } }
  | { type: 'connect'; payload: { sourceNode: string; sourcePin: string; targetNode: string; targetPin: string } }
  | { type: 'disconnect'; payload: { sourceNode: string; sourcePin: string; targetNode: string; targetPin: string } }
  | { type: 'disconnectpin'; payload: { nodeId: string; pinName: string } }
  | { type: 'nodeselect'; payload: { nodeId: string | null } }
  | { type: 'selectionchange'; payload: { nodeIds: string[] } }
  | { type: 'commentadd'; payload: { text: string; x: number; y: number; width: number; height: number } }
  | { type: 'commentremove'; payload: { commentId: string } }
  | { type: 'commentupdate'; payload: { commentId: string; text: string } }
  | { type: 'pindefaultchange'; payload: { nodeId: string; pinName: string; value: unknown } }
  | { type: 'variableadd'; payload: { name: string; dataType: string; defaultValue?: unknown } }
  | { type: 'variableremove'; payload: { name: string } }
  | { type: 'variableupdate'; payload: { name: string; dataType?: string; defaultValue?: unknown; description?: string } }
  // ── Contract events (INPUT / RETURN / 统一变量) ─────────────────────
  // 由右侧 ContractDrawer 触发，对应后端 draft_declare_* / draft_update_contract / ... 命令
  | { type: 'inputdeclare'; payload: { name: string; dataType: string; comment?: string } }
  | { type: 'inputupdate'; payload: { name: string; dataType?: string; defaultValue?: unknown; comment?: string } }
  | { type: 'inputremove'; payload: { name: string } }
  | { type: 'inputrename'; payload: { oldName: string; newName: string } }
  | { type: 'returndeclare'; payload: { name: string; dataType: string; comment?: string } }
  | { type: 'returnupdate'; payload: { name: string; dataType?: string; defaultValue?: unknown; comment?: string } }
  | { type: 'returnremove'; payload: { name: string } }
  | { type: 'returnrename'; payload: { oldName: string; newName: string } }
  | { type: 'vardeclare'; payload: { name: string; dataType: string; defaultValue?: unknown; comment?: string } }
  | { type: 'varupdate'; payload: { name: string; dataType?: string; defaultValue?: unknown; comment?: string } }
  | { type: 'varremove'; payload: { name: string } }
  | { type: 'varrename'; payload: { oldName: string; newName: string } };

// ─── Editor props ───

export type EditorMode = 'normal' | 'professional';

export interface EditorProps {
  mode: EditorMode;
  blueprintData?: BlueprintJson;
  flowchartData?: FlowchartData;
  readOnly?: boolean;
  onEvent?: (event: EditorEvent) => void;
  /** node_type → description template (for professional mode) */
  nodeDescriptions?: Record<string, string>;
  /** 普通模式：当前 AI 正在执行的步骤 ID（对应 FlowchartNode.step_id 或 id），用于高亮 */
  currentStepId?: string;
  /** 普通模式：每个节点的执行状态（key 是 FlowchartNode.id，不是 step_id） */
  stepStatuses?: Record<string, FlowchartStepStatus>;
}
