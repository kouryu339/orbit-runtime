let runtime,
  initializing = false;
let sequence = 0;
const storageRequests = new Map();
function host(message) {
  self.postMessage(message);
}
function commit(snapshot) {
  const id = `storage-${++sequence}`;
  return new Promise((resolve, reject) => {
    storageRequests.set(id, { resolve, reject });
    host({
      type: "host.request",
      id,
      operation: "resources.commit",
      payload: snapshot,
    });
  });
}
async function invoke(envelope) {
  const result = JSON.parse(await runtime.invoke(JSON.stringify(envelope)));
  if (!result.ok)
    throw Object.assign(new Error(result.error.message), result.error);
  if (
    ["workflow.register", "workflow.remove", "workflow.promote"].includes(
      envelope.type,
    )
  ) {
    try {
      await commit(JSON.parse(runtime.resource_snapshot()));
      runtime.finish_resource_commit(true);
    } catch (error) {
      runtime.finish_resource_commit(false);
      throw Object.assign(error, { kind: "storage_error" });
    }
  }
  return result.result;
}
self.onmessage = async ({ data: message }) => {
  if (message.operation === "host.complete") {
    const pending = storageRequests.get(message.id);
    if (pending) {
      storageRequests.delete(message.id);
      message.response.ok
        ? pending.resolve(message.response.value)
        : pending.reject(new Error(message.response.error));
    } else
      runtime?.complete_host_request(
        message.id,
        JSON.stringify(message.response),
      );
    return;
  }
  try {
    let result;
    if (message.operation === "initialize") {
      if (runtime || initializing)
        throw new Error("Worker already initialized");
      initializing = true;
      const wasm = message.payload.wasmModuleUrl
        ? await import(/* @vite-ignore */ message.payload.wasmModuleUrl)
        : await import("./wasm/orbit_runtime_wasm.js");
      await wasm.default();
      runtime = await wasm.create_runtime(
        JSON.stringify(message.payload.options),
        (event) => host({ type: "event", event: JSON.parse(event) }),
        (raw) => host(JSON.parse(raw)),
      );
      const registration = message.payload.registration;
      await invoke({
        schema: "agent-runtime-command/v1",
        type: "runtime.register_resources",
        payload: {
          schema: "agent-runtime-resource-registration/v1",
          skills: { sources: registration.skills },
          workflows: { sources: registration.workflows },
        },
      });
      if (registration.llm)
        await invoke({
          schema: "agent-runtime-command/v1",
          type: "runtime.register_llm",
          payload: registration.llm,
        });
      for (const cluster of registration.clusters)
        await invoke({
          schema: "agent-runtime-command/v1",
          type: "runtime.register_agent_cluster",
          payload: cluster,
        });
      for (const endpoint of registration.endpoints)
        await invoke({
          schema: "agent-runtime-command/v1",
          type: "runtime.add_rpc_endpoint",
          payload: { schema: "agent-runtime-rpc-endpoint-add/v1", endpoint },
        });
      await runtime.start();
      await commit(JSON.parse(runtime.resource_snapshot()));
      result = {
        version: runtime.version(),
        capabilities: JSON.parse(runtime.capabilities()),
      };
    } else {
      if (!runtime) throw new Error("Runtime not initialized");
      if (message.operation === "invoke")
        result = await invoke(message.payload);
      else if (message.operation === "shutdown") {
        await runtime.shutdown();
        result = {};
      } else throw new Error("Unknown SDK operation");
    }
    host({ id: message.id, ok: true, result });
  } catch (error) {
    host({
      id: message.id,
      ok: false,
      error: {
        kind: error.kind ?? "runtime_error",
        message: String(error.message ?? error),
      },
    });
  }
};
