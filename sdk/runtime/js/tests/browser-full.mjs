import { createRuntime, memoryStore } from "../dist/index.js";
const check = (value, message) => {
  if (!value) throw new Error(message);
};
const source = (id, logic, revision = 1) =>
  JSON.stringify({
    schema: "orbit.workflow.resource/v1",
    id,
    metadata: { name: id, description: "Browser fixture", extensions: {} },
    state: "draft",
    revision,
    logic: { schema: "orbit.workflow.logic/v1", steps: [], ...logic },
  });
const metadata = {
  name: "WebEcho",
  description: "Echo a value",
  readonly: true,
  idempotent: true,
  parameters: [{ name: "value", param_type: "String", required: true }],
  outputs: [{ name: "value", field_type: "String" }],
};
const role =
  "---\nname: browser-role\ndescription: Browser test employee\nkind: role\ntools: [WebEcho]\n---\nUse WebEcho once, then reply briefly.";
const llm = {
  schema: "agent-runtime-llm-registration/v1",
  id: "web-llm",
  current_model_uid: 1,
  providers: [
    {
      id: 1,
      name: "Browser fixture",
      type: "openai",
      base_url: location.origin + "/v1",
      api_key: "fixture",
      api_paradigm: "openai_chat_completions",
      enabled_models: [{ uid: 1, model_id: "gpt-4o" }],
    },
  ],
};
const cluster = {
  schema: "agent-runtime-agent-cluster-registration/v1",
  id: "web-team",
  focus_agent_id: "lead",
  agents: [
    { id: "lead", role: "browser-role", model_uid: 1 },
    { id: "helper", role: "browser-role", model_uid: 1 },
  ],
};
async function until(operation, predicate, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await operation();
    if (predicate(value)) return value;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw Error("Timed out waiting for browser Agent");
}
async function rejects(promise, contains) {
  try {
    await promise;
  } catch (error) {
    check(!contains || error.message.includes(contains), error.stack);
    return;
  }
  throw Error("Expected rejection");
}

export async function run() {
  const checks = [];
  let hostCalls = 0,
    slow = false,
    cancelled = false;
  const host = async (operation, payload, { signal }) => {
    if (operation === "rpc.list_tools")
      return { schema: "corework-agent-tool/v1", tools: [metadata] };
    if (operation === "rpc.execute") {
      hostCalls++;
      if (slow)
        await new Promise((resolve, reject) => {
          signal.addEventListener(
            "abort",
            () => {
              cancelled = true;
              reject(Error("cancelled"));
            },
            { once: true },
          );
        });
      return {
        result: { value: "browser" },
        to_ai: "Echo complete",
        error_code: 0,
      };
    }
    throw Error("unexpected host operation");
  };
  const runtime = await createRuntime({
    storage: "memory",
    skills: [role],
    clusters: [cluster],
    llm,
    host,
  });
  try {
    check(
      runtime.capabilities.platform === "browser-wasm",
      "WASM capabilities",
    );
    checks.push("worker WASM initialized");
    const defs = await runtime.invoke("runtime.get_tool_definitions");
    check(defs.tools.length > 0, "static tools missing");
    checks.push(`shared tool catalog (${defs.tools.length})`);
    const nodes = await runtime.invoke("runtime.get_workflow_node_definitions");
    check(
      nodes.nodes.some((node) => node.node_type === "StartNode"),
      "built-in nodes missing",
    );
    await runtime.invoke("runtime.add_rpc_endpoint", {
      schema: "agent-runtime-rpc-endpoint-add/v1",
      endpoint: { id: "web", protocol: "host", timeout_ms: 5000 },
    });
    await rejects(
      runtime.invoke("runtime.add_rpc_endpoint", {
        schema: "agent-runtime-rpc-endpoint-add/v1",
        endpoint: { id: "web", protocol: "host" },
      }),
      "rpc_endpoint_exists",
    );
    checks.push("append-only host tool discovery");
    const text = source("web-flow", {
      steps: [{ id: 1, call: "WebEcho", args: { value: "browser" } }],
      returns: { answer: { step: 1, output: "value" } },
    });
    const validated = await runtime.invoke("workflow.validate", {
      source: text,
    });
    check(validated.valid, "valid Workflow");
    await runtime.invoke("workflow.register", { source: text });
    const executed = await runtime.invoke("workflow.execute", {
      id: "web-flow",
    });
    check(
      executed.code === 0 && executed.result.outputs.answer === "browser",
      JSON.stringify(executed),
    );
    checks.push("workflow compile + host tool execution");
    const loop = source("control-flow", {
      variables: { count: { type: "i64", initial: 0 } },
      steps: [
        {
          id: 1,
          for: [1, 2, 3],
          body: [
            {
              id: 2,
              set: "count",
              value: { fn: "add", args: [{ var: "count" }, 1] },
            },
          ],
        },
        {
          id: 3,
          switch: "yes",
          cases: [{ value: "yes", body: [{ id: 4, set: "count", value: 9 }] }],
          default: [],
        },
      ],
      returns: { count: { var: "count" } },
    });
    await runtime.invoke("workflow.register", { source: loop });
    const controlled = await runtime.invoke("workflow.execute", {
      id: "control-flow",
    });
    check(controlled.result.outputs.count === 9, JSON.stringify(controlled));
    checks.push("shared for/switch/pure/variables");
    const info = await runtime.invoke("conversation.spawn", {
      cluster_id: "web-team",
    });
    const id = info.conversation_id;
    let snapshot,
      events = [];
    const off = runtime.subscribe((event) => {
      events.push(event);
      if (event.type === "frontend:state_snapshot") snapshot = event.payload;
    });
    const accepted = await runtime.invoke("conversation.send_message", {
      conversation_id: id,
      content: "Echo once",
    });
    check(accepted.decision === "accepted", "send admission");
    const materialized = await until(
      () => runtime.invoke("conversation.materialize", { conversation_id: id }),
      (value) =>
        value.ledger.some((record) =>
          String(record.content).includes("Browser agent finished."),
        ),
    );
    check(
      materialized.ledger.some((record) => record.role === "tool"),
      "tool ledger",
    );
    check(
      events.some(
        (event) =>
          event.type === "conversation.ledger_delta" &&
          event.payload.kind === "replace" &&
          event.payload.records.some((record) =>
            String(record.content).includes("Browser agent finished."),
          ),
      ),
      "materialized history publication",
    );
    check(snapshot.agents.length === 2, "cluster members");
    checks.push("Agent SSE + native tool calling + two-member cluster");
    await rejects(
      runtime.invoke("workflow.register", {
        source: source("web-flow", { returns: { answer: 1 } }),
      }),
      "workflow_in_use",
    );
    checks.push("live conversation resource write guard");
    const permission = await runtime.invoke("conversation.spawn", {
      cluster_id: "web-team",
      tool_permissions: {
        read_only: "ask",
        controlled_change: "ask",
        destructive: "ask",
      },
    });
    await runtime.invoke("conversation.send_message", {
      conversation_id: permission.conversation_id,
      content: "Request approval",
    });
    await until(
      () =>
        runtime.invoke("conversation.materialize", {
          conversation_id: permission.conversation_id,
        }),
      () =>
        snapshot?.conversation_id === permission.conversation_id &&
        snapshot.pending_permissions?.length > 0,
    );
    const pending = snapshot.pending_permissions[0];
    const beforeApproval = hostCalls;
    await runtime.invoke("conversation.resolve_tool_permission", {
      conversation_id: permission.conversation_id,
      tool_call_id: pending.tool_call_id,
      decision: "allow",
    });
    await until(
      () =>
        runtime.invoke("conversation.materialize", {
          conversation_id: permission.conversation_id,
        }),
      (value) =>
        value.ledger.some((record) =>
          String(record.content).includes("Browser agent finished."),
        ),
    );
    check(hostCalls === beforeApproval + 1, "approval executed once");
    await runtime.invoke("conversation.close", {
      conversation_id: permission.conversation_id,
    });
    checks.push("tool approval round trip");
    const count = hostCalls;
    slow = true;
    const second = await runtime.invoke("conversation.spawn", {
      cluster_id: "web-team",
    });
    await runtime.invoke("conversation.send_message", {
      conversation_id: second.conversation_id,
      content: "Wait for tool",
    });
    await until(
      async () => hostCalls,
      (value) => value > count,
    );
    await runtime.invoke("conversation.pause", {
      conversation_id: second.conversation_id,
    });
    await runtime.invoke("conversation.close", {
      conversation_id: second.conversation_id,
    });
    await until(async () => cancelled, Boolean);
    checks.push("pause/close responsive during host tool + cancellation");
    await runtime.invoke("conversation.close", { conversation_id: id });
    off();
    check(
      events.some((event) => event.type === "frontend:state_snapshot"),
      "frontend events",
    );
    check(
      events
        .filter((event) => event.conversation_id)
        .every((event) => event.conversation_event_seq > 0),
      "conversation event sequencing",
    );
    await runtime.invoke("workflow.remove", {
      id: "web-flow",
      expected_revision: 1,
    });
    checks.push("workflow lifecycle");
  } finally {
    await runtime.dispose();
  }

  const namespace = "wasm-test-" + crypto.randomUUID();
  const persisted = await createRuntime({ namespace });
  try {
    await persisted.invoke("workflow.register", {
      source: source("saved", { returns: { value: 7 } }),
    });
    await rejects(createRuntime({ namespace }), "already in use");
    checks.push("namespace ownership lock");
  } finally {
    await persisted.dispose();
  }
  const restored = await createRuntime({ namespace });
  try {
    check(
      (await restored.invoke("workflow.list")).workflows.length === 1,
      "persisted reload",
    );
    checks.push("IndexedDB reload");
  } finally {
    await restored.dispose();
  }

  const backing = memoryStore();
  let fail = false;
  const guarded = await createRuntime({
    namespace: "failure-" + crypto.randomUUID(),
    storage: {
      ...backing,
      save: async (...args) => {
        if (fail) throw Error("disk full fixture");
        return backing.save(...args);
      },
    },
  });
  try {
    let events = 0;
    guarded.subscribe((event) => {
      if (event.type === "workflow.resource_changed") events++;
    });
    fail = true;
    await rejects(
      guarded.invoke("workflow.register", {
        source: source("rollback", { returns: { value: 1 } }),
      }),
      "disk full",
    );
    check(
      (await guarded.invoke("workflow.list")).workflows.length === 0,
      "catalog rollback",
    );
    check(events === 0, "uncommitted event leaked");
    fail = false;
    await guarded.invoke("workflow.register", {
      source: source("rollback", { returns: { value: 1 } }),
    });
    check(events === 1, "committed event");
    checks.push("storage failure rollback + event commit");
  } finally {
    await guarded.dispose();
  }
  const { openExample } = await import("../examples/quickstart.js");
  const example = await openExample({
    baseUrl: location.origin + "/v1",
    apiKey: "fixture",
    modelId: "gpt-4o",
  });
  try {
    const manifest = await (await fetch("../dist/manifest.json")).json();
    check(
      example.runtime.version === manifest.version,
      "SDK and WASM versions differ",
    );
    await example.send("Echo browser");
    await until(
      () =>
        example.runtime.invoke("conversation.materialize", {
          conversation_id: example.conversation_id,
        }),
      (value) =>
        value.ledger.some((record) =>
          String(record.content).includes("Browser agent finished."),
        ),
    );
    checks.push("packaged quickstart Agent + host tool + version alignment");
  } finally {
    await example.close();
  }
  return { ok: true, checks };
}
