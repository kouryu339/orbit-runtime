import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import grpc from "@grpc/grpc-js";
import protoLoader from "@grpc/proto-loader";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";
import {
  MemoryEventSource,
  eventSourceService,
  serve,
  registerTool,
} from "../src/index.js";

const types = [
  {
    name: "message.received",
    schema_version: "1",
    data_schema: { type: "object" },
  },
];
test("packaged event protocol matches canonical protocol", async () => {
  assert.equal(
    await readFile(
      new URL("../proto/corework_event_source_v1.proto", import.meta.url),
      "utf8",
    ),
    await readFile(
      new URL(
        "../../../../corework/proto/corework_event_source_v1.proto",
        import.meta.url,
      ),
      "utf8",
    ),
  );
});
test("idle live gRPC subscription confirms headers before heartbeat", async () => {
  const source = new MemoryEventSource({
    sourceId: "idle-live",
    eventTypes: types,
  });
  const rpc = new grpc.Server();
  const { service, implementation } = await eventSourceService(source);
  rpc.addService(service, implementation);
  const port = await new Promise((resolve, reject) =>
    rpc.bindAsync(
      "127.0.0.1:0",
      grpc.ServerCredentials.createInsecure(),
      (e, p) => (e ? reject(e) : resolve(p)),
    ),
  );
  const definition = await protoLoader.load(
    fileURLToPath(
      new URL("../proto/corework_event_source_v1.proto", import.meta.url),
    ),
    { keepCase: true, defaults: true },
  );
  const Client =
    grpc.loadPackageDefinition(definition).corework.event_source.v1
      .EventSourceService;
  const client = new Client(
    `127.0.0.1:${port}`,
    grpc.credentials.createInsecure(),
  );
  const info = source.describe();
  const stream = client.Subscribe({
    ...info,
    consumer_id: "idle",
    event_types: [],
    after_cursor: "",
  });
  stream.on("error", () => {});
  let timer;
  try {
    await Promise.race([
      new Promise((resolve, reject) => {
        stream.once("metadata", resolve);
        stream.once("error", reject);
      }),
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(Error("idle stream headers timed out")),
          1000,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
    stream.cancel();
    client.close();
    rpc.forceShutdown();
  }
});
test("memory replay declares its lifetime and detects expired and reset cursors", async () => {
  const source = new MemoryEventSource({
    sourceId: "account-a",
    eventTypes: types,
    replayable: true,
    capacity: 1,
  });
  const first = source.publish("message.received", { text: "one" });
  source.publish("message.received", { text: "two" });
  source.publish("message.received", { text: "three" });
  const info = source.describe();
  assert.equal(info.durability, "memory");
  const request = {
    ...info,
    consumer_id: "test",
    event_types: [],
    after_cursor: first.cursor,
  };
  await assert.rejects(
    source.subscribe(request, { signal: new AbortController().signal }).next(),
    (e) => e.code === grpc.status.OUT_OF_RANGE,
  );
  const reset = new MemoryEventSource({
    sourceId: "account-a",
    eventTypes: types,
    replayable: true,
  });
  await assert.rejects(
    reset.subscribe(request, { signal: new AbortController().signal }).next(),
    (e) => e.code === grpc.status.FAILED_PRECONDITION,
  );
  assert.throws(() =>
    source.publish("message.received", { text: "x".repeat(65537) }),
  );
});
test("live source cannot claim replay; cancelled iterator is released", async () => {
  const source = new MemoryEventSource({ sourceId: "live", eventTypes: types });
  const info = source.describe();
  assert.equal(info.durability, "live");
  await assert.rejects(
    source
      .subscribe(
        { ...info, consumer_id: "test", event_types: [], after_cursor: "old" },
        { signal: new AbortController().signal },
      )
      .next(),
    (e) => e.code === grpc.status.INVALID_ARGUMENT,
  );
  const cancel = new AbortController();
  const iterator = source.subscribe(
    { ...info, consumer_id: "test", event_types: [], after_cursor: "" },
    { signal: cancel.signal },
  );
  const next = iterator.next();
  cancel.abort();
  assert.equal((await next).done, true);
});
test("real gRPC endpoint supports tools and events together with idempotent ack", async () => {
  const source = new MemoryEventSource({
    sourceId: "combined",
    eventTypes: types,
    replayable: true,
  });
  const first = source.publish("message.received", { text: "hello" });
  registerTool(
    {
      name: "EventFixtureTool",
      description: "Fixture",
      parameters: [],
      outputs: [],
    },
    async () => ({ result: {}, to_ai: "ok", error_code: 1 }),
  );
  const definition = await protoLoader.load(
    fileURLToPath(
      new URL("../proto/corework_event_source_v1.proto", import.meta.url),
    ),
    { keepCase: true, defaults: true, longs: String },
  );
  const reservation = createServer();
  await new Promise((r) => reservation.listen(0, "127.0.0.1", r));
  const port = reservation.address().port;
  await new Promise((r) => reservation.close(r));
  const rpc = await serve(`127.0.0.1:${port}`, { eventSource: source });
  const Client =
    grpc.loadPackageDefinition(definition).corework.event_source.v1
      .EventSourceService;
  const client = new Client(
    `127.0.0.1:${port}`,
    grpc.credentials.createInsecure(),
  );
  const toolDefinition = await protoLoader.load(
    fileURLToPath(
      new URL(
        "../../../../corework/proto/corework_agent_tool_v1.proto",
        import.meta.url,
      ),
    ),
    { keepCase: true, defaults: true },
  );
  const Tools =
    grpc.loadPackageDefinition(toolDefinition).corework.agent_tool.v1
      .AgentToolService;
  const tools = new Tools(
    `127.0.0.1:${port}`,
    grpc.credentials.createInsecure(),
  );
  const unary = (name, payload) =>
    new Promise((resolve, reject) =>
      client[name](payload, (e, v) => (e ? reject(e) : resolve(v))),
    );
  try {
    const toolCatalog = await new Promise((resolve, reject) =>
      tools.ListTools({}, (e, v) => (e ? reject(e) : resolve(v))),
    );
    assert(toolCatalog.tools.some((t) => t.name === "EventFixtureTool"));
    const catalog = await unary("ListEventTypes", {
      accepted_schema: ["corework-event-source/v1"],
    });
    const request = {
      source_id: catalog.source_id,
      stream_id: catalog.stream_id,
      consumer_id: "test",
      event_types: [],
      after_cursor: "",
    };
    const stream = client.Subscribe(request);
    stream.on("error", () => {});
    const event = await new Promise((resolve, reject) => {
      stream.once("data", resolve);
      stream.once("error", reject);
    });
    assert.equal(event.event.event_id, first.event_id);
    for (let i = 0; i < 2; i++)
      assert.equal(
        (await unary("Acknowledge", { ...request, cursor: first.cursor }))
          .cursor,
        first.cursor,
      );
    stream.cancel();
  } finally {
    tools.close();
    client.close();
    rpc.forceShutdown();
  }
});
