import { createRuntime, type BrowserRuntime } from "../src/index.js";
import { WorkerConversationTransport } from "../../../../ui/lit/src/transport/worker.js";
async function example() {
  const runtime: BrowserRuntime = await createRuntime({ storage: "memory" });
  const transport = new WorkerConversationTransport(runtime);
  await runtime.invoke("workflow.list", {});
  await runtime.dispose();
  return transport;
}
void example;
