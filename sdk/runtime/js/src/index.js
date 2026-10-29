import { acquireNamespace, indexedDBStore, memoryStore } from "./storage.js";
export { memoryStore, indexedDBStore } from "./storage.js";
export const workflowSchemas = Object.freeze({
  logic: new URL("./schemas/logic.schema.json", import.meta.url),
  resource: new URL("./schemas/resource.schema.json", import.meta.url),
});

export class RuntimeError extends Error {
  constructor(error) {
    super(error.message);
    this.name = "RuntimeError";
    this.kind = error.kind;
    this.details = error;
  }
}

async function abortable(operation, signal) {
  let cancel;
  try {
    return await Promise.race([
      operation,
      new Promise((_, reject) => {
        cancel = () =>
          reject(new DOMException("Host operation cancelled", "AbortError"));
        signal.addEventListener("abort", cancel, { once: true });
        if (signal.aborted) cancel();
      }),
    ]);
  } finally {
    signal.removeEventListener("abort", cancel);
  }
}

export async function createRuntime(options = {}) {
  const namespace = options.namespace ?? "default";
  if (
    typeof namespace !== "string" ||
    !namespace.trim() ||
    namespace.length > 128
  )
    throw new Error("namespace must be a nonempty string up to 128 characters");
  const ephemeral = options.storage === "memory";
  let unlock;
  let runtime;
  const store = ephemeral
    ? memoryStore()
    : (options.storage ?? indexedDBStore(namespace));
  try {
    if (!ephemeral) unlock = await acquireNamespace(namespace);
    const saved = await store.load();
    if (
      !Number.isSafeInteger(saved?.revision) ||
      saved.revision < 0 ||
      (saved.snapshot &&
        saved.snapshot.schema !== "orbit-runtime-browser-resources/v1")
    )
      throw new Error("Unsupported or invalid stored resource snapshot");
    const worker = options.workerUrl
      ? new Worker(options.workerUrl, {
          type: "module",
          name: `Orbit ${namespace}`,
        })
      : new Worker(new URL("./worker.js", import.meta.url), {
          type: "module",
          name: `Orbit ${namespace}`,
        });
    runtime = new BrowserRuntime(worker, store, unlock, options);
    let timer;
    try {
      await Promise.race([
        runtime.initialize(saved),
        new Promise((_, reject) => {
          timer = setTimeout(
            () => reject(new Error("Runtime initialization timed out")),
            options.initializationTimeoutMs ?? 60000,
          );
        }),
      ]);
      return runtime;
    } finally {
      clearTimeout(timer);
    }
  } catch (error) {
    if (runtime) await runtime.dispose();
    else {
      store.close?.();
      await unlock?.();
    }
    throw error;
  }
}

class BrowserRuntime {
  #worker;
  #store;
  #unlock;
  #options;
  #pending = new Map();
  #hosts = new Map();
  #listeners = new Set();
  #next = 0;
  #closed = false;
  #revision = 0;
  #writes = Promise.resolve();
  #cleanup;
  #storageWrites = new Set();
  capabilities;
  version;
  constructor(worker, store, unlock, options) {
    this.#worker = worker;
    this.#store = store;
    this.#unlock = unlock;
    this.#options = options;
    worker.onmessage = (event) => this.#receive(event.data);
    worker.onerror = (event) => {
      event.preventDefault();
      this.#fail(new Error(event.message || "Runtime Worker failed"));
    };
    worker.onmessageerror = () =>
      this.#fail(new Error("Invalid Worker message"));
  }
  async initialize(saved) {
    this.#revision = saved.revision;
    const info = await this.#request("initialize", {
      options: this.#options.runtimeOptions ?? {},
      wasmModuleUrl:
        this.#options.wasmModuleUrl &&
        new URL(this.#options.wasmModuleUrl, location.href).href,
      registration: {
        skills: this.#options.skills ?? saved.snapshot?.skills ?? [],
        workflows:
          saved.snapshot?.workflow_sources ?? this.#options.workflows ?? [],
        clusters: this.#options.clusters ?? saved.snapshot?.clusters ?? [],
        llm: this.#options.llm,
        endpoints: this.#options.endpoints ?? [],
      },
    });
    this.capabilities = Object.freeze(info.capabilities);
    this.version = info.version;
  }
  subscribe(listener) {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }
  async invoke(type, payload = {}, { commandId } = {}) {
    if (this.#closed) throw new Error("Runtime is disposed");
    const call = () =>
      this.#request("invoke", {
        schema: "agent-runtime-command/v1",
        id: ++this.#next,
        command_id: commandId ?? crypto.randomUUID(),
        type,
        payload,
      });
    // Only resource writes are serialized. Approval, pause and close remain responsive.
    if (
      ["workflow.register", "workflow.remove", "workflow.promote"].includes(
        type,
      )
    ) {
      const result = this.#writes.then(call);
      this.#writes = result.catch(() => {});
      return result;
    }
    return call();
  }
  async #request(operation, payload) {
    if (this.#closed) throw new Error("Runtime is disposed");
    if (this.#pending.size >= 256)
      throw new Error("Too many pending Runtime commands");
    const id = `sdk-${++this.#next}`;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject });
      try {
        this.#worker.postMessage({ id, operation, payload });
      } catch (error) {
        this.#pending.delete(id);
        reject(error);
      }
    });
  }
  async #receive(message) {
    if (this.#closed) return;
    if (message.type === "event") {
      for (const listener of this.#listeners) {
        try {
          listener(message.event);
        } catch (error) {
          try {
            this.#options.onError?.(error);
          } catch {
            /* Keep other observers active. */
          }
        }
      }
    } else if (message.type === "host.cancel") {
      this.#hosts.get(message.id)?.abort();
      this.#hosts.delete(message.id);
    } else if (message.type === "host.request") {
      const controller = new AbortController();
      this.#hosts.set(message.id, controller);
      try {
        let value;
        if (message.operation === "resources.commit") {
          const writing = this.#store.save(message.payload, this.#revision);
          this.#storageWrites.add(writing);
          try {
            this.#revision = await writing;
            value = { revision: this.#revision };
          } finally {
            this.#storageWrites.delete(writing);
          }
        } else {
          if (!this.#options.host)
            throw new Error(`No host adapter for ${message.operation}`);
          value = await abortable(
            this.#options.host(message.operation, message.payload, {
              signal: controller.signal,
            }),
            controller.signal,
          );
        }
        if (!controller.signal.aborted && !this.#closed)
          this.#worker.postMessage({
            operation: "host.complete",
            id: message.id,
            response: { ok: true, value },
          });
      } catch (error) {
        if (!controller.signal.aborted && !this.#closed)
          this.#worker.postMessage({
            operation: "host.complete",
            id: message.id,
            response: { ok: false, error: String(error?.message ?? error) },
          });
      } finally {
        this.#hosts.delete(message.id);
      }
    } else {
      const pending = this.#pending.get(message.id);
      if (!pending) return;
      this.#pending.delete(message.id);
      message.ok
        ? pending.resolve(message.result)
        : pending.reject(new RuntimeError(message.error));
    }
  }
  #fail(error, notify = true) {
    this.#closed = true;
    for (const pending of this.#pending.values()) pending.reject(error);
    this.#pending.clear();
    for (const controller of this.#hosts.values()) controller.abort();
    this.#hosts.clear();
    this.#worker.terminate();
    this.#cleanup ??= Promise.allSettled([...this.#storageWrites]).then(
      async () => {
        this.#store.close?.();
        await this.#unlock?.();
        this.#unlock = undefined;
      },
    );
    if (notify) {
      try {
        this.#options.onError?.(error);
      } catch {
        /* Observer failures cannot prevent cleanup. */
      }
    }
  }
  async dispose() {
    if (!this.#closed) {
      try {
        await Promise.race([
          this.#request("shutdown", {}),
          new Promise((resolve) => setTimeout(resolve, 1500)),
        ]);
      } finally {
        this.#fail(new Error("Runtime is disposed"), false);
      }
    }
    await this.#cleanup;
    this.#listeners.clear();
  }
}
