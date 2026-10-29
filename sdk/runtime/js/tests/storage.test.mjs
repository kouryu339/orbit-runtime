import { test } from "node:test";
import assert from "node:assert/strict";
import { memoryStore } from "../src/storage.js";

test("content commits compare revisions and isolate caller-owned objects", async () => {
  const store = memoryStore();
  const value = { workflow_sources: ["first"] };
  assert.equal(await store.save(value, 0), 1);
  value.workflow_sources.push("not saved");
  const saved = await store.load();
  assert.deepEqual(saved.snapshot.workflow_sources, ["first"]);
  saved.snapshot.workflow_sources.length = 0;
  await assert.rejects(
    store.save({ workflow_sources: ["stale"] }, 0),
    /revision conflict/,
  );
  assert.deepEqual((await store.load()).snapshot.workflow_sources, ["first"]);
  assert.equal(await store.save({ workflow_sources: ["second"] }, 1), 2);
});
