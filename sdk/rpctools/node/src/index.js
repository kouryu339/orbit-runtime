import { EventEmitter, once } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import grpc from "@grpc/grpc-js";
import protoLoader from "@grpc/proto-loader";

export const SCHEMA = "corework-agent-tool/v1";

export const ToolErrorCode = Object.freeze({
  UNSPECIFIED: 0,
  OK: 1,
  INVALID_ARGUMENT: 100,
  MISSING_ARGUMENT: 101,
  PERMISSION_DENIED: 102,
  NOT_FOUND: 103,
  CONFLICT: 104,
  INTERNAL: 200,
  TIMEOUT: 201,
  CANCELLED: 202,
  UNAVAILABLE: 203,
  HOST_CAPABILITY_DENIED: 300,
  HOST_CAPABILITY_UNSUPPORTED: 301,
  HOST_CALL_FAILED: 302,
  PROTOCOL_ERROR: 400,
  INVALID_OUTPUT: 401,
  SCHEMA_MISMATCH: 402,
});

export class AIOutput {
  constructor({ result = null, to_ai, error_code = ToolErrorCode.OK }) {
    this.result = result;
    this.to_ai = to_ai;
    this.error_code = error_code;
  }
}

export const registeredTools = [];

export class ToolContext {
  constructor(callId = "", hostCall = null, executeRequest = {}) {
    this.callId = callId;
    this.toolCallId = executeRequest.tool_call_id ?? "";
    this.idempotencyKey = executeRequest.idempotency_key ?? "";
    this.sessionId = executeRequest.session_id ?? "";
    this.providerId = executeRequest.provider_id ?? "";
    this.clusterId = executeRequest.cluster_id ?? "";
    this.runtimeInstanceId = executeRequest.runtime_instance_id ?? "";
    this.conversationId = executeRequest.conversation_id ?? "";
    this.agentId = executeRequest.agent_id ?? "";
    this.turnId = executeRequest.turn_id ?? "";
    this.permissions = Array.isArray(executeRequest.permissions) ? [...executeRequest.permissions] : [];
    this.hostContext = parseHostContext(executeRequest.host_context_json);
    this._hostCall = hostCall;
  }

  async workspaceResolvePath(path) {
    return this._callHost("workspace.resolve_path", { path });
  }

  async workspaceResolveWorkingPath(path) {
    return this._callHost("workspace.resolve_working_path", { path });
  }

  async workspaceCreatePath(path) {
    return this._callHost("workspace.create_path", { path });
  }

  async workspaceCreateWorkingPath(path) {
    return this._callHost("workspace.create_working_path", { path });
  }

  async workspaceSaveAsEdited(sourcePath, suffix) {
    return this._callHost("workspace.save_as_edited", {
      source_path: sourcePath,
      suffix,
    });
  }

  async _callHost(op, args) {
    if (!this._hostCall) {
      throw new Error(`host call ${op} is unavailable outside Execute`);
    }
    return this._hostCall(op, args);
  }
}

export function registerTool(metadata, handler) {
  if (!metadata?.name) {
    throw new Error("tool metadata requires a non-empty name");
  }
  registeredTools.push({ metadata, handler });
}

export async function serve(address = "127.0.0.1:50051") {
  if (typeof address === "object" && address !== null) {
    address = `${address.host ?? "127.0.0.1"}:${address.port ?? 50051}`;
  }
  const proto = await loadProto();
  const server = new grpc.Server();
  server.addService(proto.corework.agent_tool.v1.AgentToolService.service, {
    ListTools: listTools,
    Execute: execute,
  });

  await new Promise((resolve, reject) => {
    server.bindAsync(address, grpc.ServerCredentials.createInsecure(), (error) => {
      if (error) reject(error);
      else resolve();
    });
  });
  return server;
}

function listTools(call, callback) {
  const accepted = call.request?.accepted_schema ?? [];
  if (accepted.length > 0 && !accepted.includes(SCHEMA)) {
    callback({
      code: grpc.status.FAILED_PRECONDITION,
      message: `unsupported schema; expected ${SCHEMA}`,
    });
    return;
  }
  callback(null, {
    schema: SCHEMA,
    tools: registeredTools.map(({ metadata }) => descriptorToProto(metadata)),
  });
}

function execute(stream) {
  let firstMessage = null;
  const inbound = new EventEmitter();
  const pendingHostResults = [];
  let ended = false;

  stream.on("data", (message) => {
    if (!firstMessage) {
      firstMessage = message;
      runExecute(stream, firstMessage, inbound, pendingHostResults).catch((error) => {
        writeToolError(stream, firstMessage?.call_id ?? "", String(error?.message ?? error), ToolErrorCode.INTERNAL);
        stream.end();
      });
      return;
    }

    if (pendingHostResults.length > 0) {
      pendingHostResults.shift()(message);
    } else {
      inbound.emit("message", message);
    }
  });
  stream.on("end", () => {
    ended = true;
    inbound.emit("end");
  });
  stream.on("error", () => {
    ended = true;
    inbound.emit("end");
  });

  inbound.isEnded = () => ended;
}

async function runExecute(stream, first, inbound, pendingHostResults) {
  const executeRequest = first.execute_request;
  if (!executeRequest?.tool_name) {
    writeToolError(stream, first.call_id ?? "", "first stream message must be ExecuteRequest", ToolErrorCode.PROTOCOL_ERROR);
    stream.end();
    return;
  }

  const tool = registeredTools.find(({ metadata }) => metadata.name === executeRequest.tool_name);
  if (!tool) {
    writeToolError(stream, first.call_id ?? "", `unknown tool ${executeRequest.tool_name}`, ToolErrorCode.NOT_FOUND);
    stream.end();
    return;
  }

  let args;
  try {
    args = requestArgs(tool.metadata, executeRequest);
  } catch (error) {
    writeToolError(stream, first.call_id ?? "", String(error?.message ?? error), ToolErrorCode.INVALID_ARGUMENT);
    stream.end();
    return;
  }

  const ctx = new ToolContext(first.call_id ?? "", (op, hostArgs) =>
    sendHostCall(stream, first.call_id ?? "", op, hostArgs, inbound, pendingHostResults),
    executeRequest
  );

  try {
    const output = await tool.handler(ctx, args);
    if (!(output instanceof AIOutput) && !isAIOutputShape(output)) {
      throw new Error("tool handler must return AIOutput");
    }
    if (!String(output.to_ai ?? "").trim()) {
      writeToolError(stream, first.call_id ?? "", "AIOutput.to_ai must be non-empty", ToolErrorCode.INVALID_OUTPUT);
      stream.end();
      return;
    }
    stream.write({
      call_id: first.call_id ?? "",
      ai_output: {
        result_json: JSON.stringify(output.result ?? null),
        to_ai: output.to_ai,
        error_code: output.error_code ?? ToolErrorCode.OK,
      },
    });
  } catch (error) {
    writeToolError(stream, first.call_id ?? "", String(error?.message ?? error), ToolErrorCode.INTERNAL);
  } finally {
    stream.end();
  }
}

export function requestArgs(metadata, executeRequest) {
  const argsJson = String(executeRequest.args_json ?? "").trim();
  if (argsJson) {
    const args = JSON.parse(argsJson);
    if (args === null || Array.isArray(args) || typeof args !== "object") {
      throw new Error("args_json must encode an object");
    }
    return args;
  }

  const argsCli = String(executeRequest.args_cli ?? "").trim();
  if (argsCli) {
    return argsFromCli(metadata, executeRequest.tool_name, argsCli);
  }

  return {};
}

function argsFromCli(metadata, toolName, argsCli) {
  const parsed = parseCliArgs(argsCli);
  const args = {};
  for (const parameter of metadata.parameters ?? []) {
    const name = parameter.name;
    if (!name) continue;
    if (Object.prototype.hasOwnProperty.call(parsed, name)) {
      args[name] = parsed[name];
    } else if (parameter.default_value !== undefined && parameter.default_value !== null) {
      args[name] = parameter.default_value;
    } else if (parameter.required) {
      throw new Error(`missing required argument '${name}' for tool '${toolName}'`);
    }
  }
  return args;
}

function parseCliArgs(input) {
  const tokens = tokenizeCli(input);
  const args = {};
  for (let i = 0; i < tokens.length; i += 1) {
    const token = tokens[i];
    if (!token.startsWith("--") || token.length <= 2) continue;
    const raw = token.slice(2);
    const eq = raw.indexOf("=");
    if (eq >= 0) {
      args[raw.slice(0, eq)] = raw.slice(eq + 1);
    } else if (i + 1 < tokens.length && !tokens[i + 1].startsWith("--")) {
      args[raw] = tokens[++i];
    } else {
      args[raw] = "true";
    }
  }
  return args;
}

function parseHostContext(raw) {
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
}

function tokenizeCli(input) {
  const tokens = [];
  let current = "";
  let quote = null;
  for (let i = 0; i < input.length; i += 1) {
    const ch = input[i];
    if (quote) {
      if (ch === quote) {
        quote = null;
      } else if (ch === "\\" && i + 1 < input.length) {
        const escaped = input[i + 1];
        if (escaped === quote || escaped === "\\") {
          current += escaped;
          i += 1;
        } else {
          current += ch;
        }
      } else {
        current += ch;
      }
      continue;
    }
    if (ch === "\"" || ch === "'") {
      quote = ch;
    } else if (/\s/.test(ch)) {
      if (current) {
        tokens.push(current);
        current = "";
      }
    } else {
      current += ch;
    }
  }
  if (current) tokens.push(current);
  return tokens;
}

async function sendHostCall(stream, callId, op, args, inbound, pendingHostResults) {
  const id = `${callId}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  stream.write({
    call_id: callId,
    host_call: {
      id,
      op,
      args_json: JSON.stringify(args ?? {}),
    },
  });

  const message = await nextHostResult(inbound, pendingHostResults);
  if ((message.call_id ?? "") !== callId || !message.host_result) {
    throw new Error("invalid HostResult message");
  }
  const result = message.host_result;
  if (result.id !== id) {
    throw new Error(`HostResult id mismatch: ${result.id}`);
  }

  let value;
  try {
    value = JSON.parse(result.value_json || "null");
  } catch {
    value = result.value_json;
  }
  if (result.ok) {
    return value;
  }
  throw new Error(`host call ${op} failed with code ${result.code}: ${JSON.stringify(value)}`);
}

async function nextHostResult(inbound, pendingHostResults) {
  if (inbound.isEnded()) {
    throw new Error("stream closed before HostResult");
  }
  return new Promise((resolve, reject) => {
    pendingHostResults.push(resolve);
    once(inbound, "end").then(() => reject(new Error("stream closed before HostResult")));
  });
}

function descriptorToProto(metadata) {
  return {
    name: metadata.name,
    description: metadata.description ?? "",
    parameters: (metadata.parameters ?? []).map((item) => ({
      name: item.name ?? "",
      param_type: item.param_type ?? "",
      required: Boolean(item.required),
      default_value: item.default_value == null ? undefined : String(item.default_value),
      description: item.description ?? "",
    })),
    outputs: (metadata.outputs ?? []).map((item) => ({
      name: item.name ?? "",
      field_type: item.field_type ?? "",
      description: item.description ?? "",
    })),
    destructive: Boolean(metadata.destructive),
    readonly: Boolean(metadata.readonly),
    idempotent: Boolean(metadata.idempotent),
    open_world: Boolean(metadata.open_world),
    secret: Boolean(metadata.secret),
    category: metadata.category ?? "",
    display_name: metadata.display_name ?? "",
    required_capabilities: metadata.required_capabilities ?? [],
  };
}

function writeToolError(stream, callId, message, code) {
  stream.write({
    call_id: callId,
    error: {
      message,
      code,
    },
  });
}

function isAIOutputShape(value) {
  return value && typeof value === "object" && "result" in value && "to_ai" in value;
}

async function loadProto() {
  const protoPath = findProtoPath();
  const packageDefinition = await protoLoader.load(protoPath, {
    keepCase: true,
    longs: String,
    enums: Number,
    defaults: true,
    oneofs: true,
  });
  return grpc.loadPackageDefinition(packageDefinition);
}

function findProtoPath() {
  if (process.env.COREWORK_AGENT_TOOL_PROTO) {
    return process.env.COREWORK_AGENT_TOOL_PROTO;
  }
  const here = path.dirname(fileURLToPath(import.meta.url));
  const repoProto = path.resolve(here, "../../../../corework/proto/corework_agent_tool_v1.proto");
  if (fs.existsSync(repoProto)) {
    return repoProto;
  }
  const tmpProto = path.join(os.tmpdir(), "corework_agent_tool_v1.proto");
  if (fs.existsSync(tmpProto)) {
    return tmpProto;
  }
  throw new Error("Corework proto not found; set COREWORK_AGENT_TOOL_PROTO");
}
