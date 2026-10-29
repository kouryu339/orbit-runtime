export type Json =
  | null
  | boolean
  | number
  | string
  | Json[]
  | { [key: string]: Json };
export interface ResourceSnapshot {
  schema: "orbit-runtime-browser-resources/v1";
  workflow_sources: string[];
  skills: string[];
  clusters: Json[];
}
export interface ResourceStore {
  load(): Promise<{ revision: number; snapshot: ResourceSnapshot | null }>;
  save(snapshot: ResourceSnapshot, expectedRevision: number): Promise<number>;
  close?(): void;
}
export interface RuntimeEvent {
  schema: "agent-runtime-event/v1";
  type: string;
  payload: Json;
  conversation_id?: string;
  event_seq: number;
  [key: string]: unknown;
}
export interface Capabilities {
  schema: string;
  platform: "browser-wasm";
  commands: string[];
  unsupported: string[];
  [key: string]: unknown;
}
export interface RuntimeOptions {
  namespace?: string;
  /** Defaults to IndexedDB. Custom persistent adapters also require a Web Lock. */
  storage?: "memory" | ResourceStore;
  workerUrl?: string | URL;
  wasmModuleUrl?: string | URL;
  initializationTimeoutMs?: number;
  runtimeOptions?: { language?: "zh" | "en" };
  skills?: string[];
  workflows?: string[];
  clusters?: Json[];
  llm?: Json;
  endpoints?: { id: string; protocol: "host"; timeout_ms?: number }[];
  host?(
    operation: "rpc.list_tools" | "rpc.execute" | string,
    payload: any,
    context: { signal: AbortSignal },
  ): Promise<Json>;
  onError?(error: Error): void;
}
export interface BrowserRuntime {
  readonly version: string;
  readonly capabilities: Readonly<Capabilities>;
  invoke<T = any>(
    type: string,
    payload?: Json,
    options?: { commandId?: string },
  ): Promise<T>;
  subscribe(listener: (event: RuntimeEvent) => void): () => void;
  dispose(): Promise<void>;
}
export class RuntimeError extends Error {
  constructor(error: { kind: string; message: string; [key: string]: Json });
  readonly kind: string;
  readonly details: Json;
}
export function createRuntime(
  options?: RuntimeOptions,
): Promise<BrowserRuntime>;
export function memoryStore(): ResourceStore;
export const workflowSchemas: Readonly<{ logic: URL; resource: URL }>;
export function indexedDBStore(namespace: string): ResourceStore;
