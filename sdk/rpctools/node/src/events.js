import grpc from "@grpc/grpc-js";
import protoLoader from "@grpc/proto-loader";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";

export const EVENT_SOURCE_SCHEMA = "corework-event-source/v1";
export async function serveEventSource(address, source) {
  const server = new grpc.Server();
  const { service, implementation } = await eventSourceService(source);
  server.addService(service, implementation);
  await new Promise((resolve, reject) =>
    server.bindAsync(
      address,
      grpc.ServerCredentials.createInsecure(),
      (error) => (error ? reject(error) : resolve()),
    ),
  );
  return server;
}
const protoPath =
  process.env.COREWORK_EVENT_SOURCE_PROTO ??
  fileURLToPath(
    new URL("../proto/corework_event_source_v1.proto", import.meta.url),
  );
export async function eventSourceService(source) {
  const definition = await protoLoader.load(protoPath, {
    keepCase: true,
    longs: String,
    defaults: true,
    oneofs: true,
  });
  const service =
    grpc.loadPackageDefinition(definition).corework.event_source.v1
      .EventSourceService.service;
  return {
    service,
    implementation: {
      async ListEventTypes(call, callback) {
        try {
          if (
            call.request.accepted_schema.length &&
            !call.request.accepted_schema.includes(EVENT_SOURCE_SCHEMA)
          )
            throw status(
              grpc.status.INVALID_ARGUMENT,
              "Unsupported event schema",
            );
          callback(null, await source.describe());
        } catch (e) {
          callback(normalize(e));
        }
      },
      Subscribe(call) {
        // Confirm idle live subscriptions before the first event/heartbeat.
        call.sendMetadata(new grpc.Metadata());
        const controller = new AbortController();
        call.on("cancelled", () => controller.abort());
        const heartbeat = setInterval(() => {
          if (!call.writableNeedDrain && !controller.signal.aborted)
            call.write({ heartbeat: {} });
        }, 5000);
        heartbeat.unref();
        (async () => {
          try {
            for await (const event of source.subscribe(call.request, {
              signal: controller.signal,
            })) {
              if (controller.signal.aborted) break;
              if (!call.write({ event }))
                await drained(call, controller.signal);
            }
            if (!controller.signal.aborted) call.end();
          } catch (e) {
            if (!controller.signal.aborted) call.destroy(normalize(e));
          } finally {
            clearInterval(heartbeat);
            controller.abort();
          }
        })();
      },
      async Acknowledge(call, callback) {
        try {
          callback(null, await source.acknowledge(call.request));
        } catch (e) {
          callback(normalize(e));
        }
      },
    },
  };
}
function normalize(e) {
  return status(
    Number.isInteger(e.code) ? e.code : grpc.status.INTERNAL,
    "Event source request failed",
  );
}
function status(code, message) {
  return Object.assign(new Error(message), { code });
}
function drained(call, signal) {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      call.off("drain", done);
      call.off("error", failed);
      signal.removeEventListener("abort", cancel);
    };
    const done = () => {
      cleanup();
      resolve();
    };
    const failed = (e) => {
      cleanup();
      reject(e);
    };
    const cancel = () => failed(status(grpc.status.CANCELLED, "Cancelled"));
    call.once("drain", done);
    call.once("error", failed);
    signal.addEventListener("abort", cancel, { once: true });
    if (signal.aborted) cancel();
  });
}

/** Bounded reference journal. Memory replay lasts only this process/stream.
 * Durable sources implement describe/subscribe/acknowledge against their own DB.
 */
export class MemoryEventSource {
  #events = [];
  #head = 0;
  #waiters = new Set();
  #types;
  #source;
  #stream;
  #replay;
  #capacity;
  #retention;
  constructor({
    sourceId,
    eventTypes,
    replayable = false,
    capacity = 256,
    retentionMs = 300000,
  }) {
    if (
      typeof sourceId !== "string" ||
      !sourceId.trim() ||
      sourceId.length > 128
    )
      throw Error("sourceId required");
    if (
      !Number.isSafeInteger(capacity) ||
      capacity < 1 ||
      capacity > 4096 ||
      !Number.isSafeInteger(retentionMs) ||
      retentionMs < 1
    )
      throw Error("Invalid retention bounds");
    if (
      !Array.isArray(eventTypes) ||
      !eventTypes.length ||
      eventTypes.length > 128 ||
      new Set(eventTypes.map((e) => e.name)).size !== eventTypes.length
    )
      throw Error("Unique event types required");
    this.#types = eventTypes.map((e) => {
      if (
        !e.name?.trim() ||
        !e.schema_version?.trim() ||
        !e.data_schema ||
        typeof e.data_schema !== "object" ||
        Array.isArray(e.data_schema)
      )
        throw Error("Invalid event descriptor");
      return {
        name: e.name,
        description: e.description ?? "",
        schema_version: e.schema_version,
        data_schema_json: JSON.stringify(e.data_schema),
      };
    });
    this.#source = sourceId;
    this.#stream = randomUUID();
    this.#replay = Boolean(replayable);
    this.#capacity = capacity;
    this.#retention = retentionMs;
  }
  describe() {
    return {
      schema: EVENT_SOURCE_SCHEMA,
      source_id: this.#source,
      stream_id: this.#stream,
      event_types: this.#types.map((e) => ({ ...e })),
      replayable: this.#replay,
      durability: this.#replay ? "memory" : "live",
      retention_ms: this.#replay ? this.#retention : 0,
    };
  }
  publish(eventType, data) {
    const t = this.#types.find((t) => t.name === eventType);
    if (!t) throw Error("Unknown event type");
    const data_json = JSON.stringify(data);
    if (typeof data_json !== "string" || Buffer.byteLength(data_json) > 65536)
      throw Error("Event JSON exceeds 64 KiB");
    const event = {
      source_id: this.#source,
      stream_id: this.#stream,
      event_id: randomUUID(),
      event_type: eventType,
      schema_version: t.schema_version,
      occurred_at: new Date().toISOString(),
      cursor: `${this.#stream}:${++this.#head}`,
      data_json,
    };
    this.#events.push({ seq: this.#head, time: Date.now(), event });
    this.#prune();
    for (const wake of this.#waiters) wake();
    return { ...event };
  }
  #prune() {
    while (
      this.#events.length &&
      (this.#events.length > this.#capacity ||
        this.#events[0].time < Date.now() - this.#retention)
    )
      this.#events.shift();
  }
  #identity(r) {
    if (r.source_id !== this.#source || r.stream_id !== this.#stream)
      throw status(grpc.status.FAILED_PRECONDITION, "Source changed");
    if (!r.consumer_id?.trim() || r.consumer_id.length > 128)
      throw status(grpc.status.INVALID_ARGUMENT, "consumer_id required");
  }
  #sequence(cursor) {
    if (!cursor.startsWith(this.#stream + ":"))
      throw status(grpc.status.FAILED_PRECONDITION, "Journal reset");
    const tail = cursor.slice(this.#stream.length + 1);
    const n = Number(tail);
    if (
      !/^\d+$/.test(tail) ||
      !Number.isSafeInteger(n) ||
      n < 0 ||
      n > this.#head
    )
      throw status(grpc.status.INVALID_ARGUMENT, "Invalid cursor");
    return n;
  }
  async *subscribe(r, { signal }) {
    this.#identity(r);
    this.#prune();
    if (r.event_types.some((n) => !this.#types.some((t) => t.name === n)))
      throw status(grpc.status.INVALID_ARGUMENT, "Unknown event type");
    if (!this.#replay && r.after_cursor)
      throw status(grpc.status.INVALID_ARGUMENT, "Live source cannot replay");
    let position = r.after_cursor
      ? this.#sequence(r.after_cursor)
      : this.#replay
        ? (this.#events[0]?.seq ?? this.#head + 1) - 1
        : this.#head;
    while (!signal.aborted) {
      this.#prune();
      const oldest = this.#events[0]?.seq ?? this.#head + 1;
      if (position < oldest - 1)
        throw status(grpc.status.OUT_OF_RANGE, "Cursor expired");
      const next = this.#events.find((e) => e.seq > position);
      if (next) {
        position = next.seq;
        if (
          !r.event_types.length ||
          r.event_types.includes(next.event.event_type)
        )
          yield { ...next.event };
        continue;
      }
      await new Promise((resolve) => {
        const wake = () => {
          this.#waiters.delete(wake);
          signal.removeEventListener("abort", wake);
          resolve();
        };
        this.#waiters.add(wake);
        signal.addEventListener("abort", wake, { once: true });
        if (signal.aborted) wake();
      });
    }
  }
  acknowledge(r) {
    this.#identity(r);
    this.#sequence(r.cursor);
    return { cursor: r.cursor };
  }
}
