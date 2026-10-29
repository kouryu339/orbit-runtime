// Use from a browser ES module after serving this package over HTTPS / localhost.
import { createRuntime } from "../dist/index.js";

export async function openExample({
  baseUrl,
  apiKey,
  modelId,
  onEvent = () => {},
}) {
  const role = `---
name: browser-assistant
description: 浏览器助手
kind: role
tools: [WebEcho]
---
按用户要求使用 WebEcho 返回文字，然后简短回复。`;
  const runtime = await createRuntime({
    namespace: "example-project",
    skills: [role],
    llm: {
      schema: "agent-runtime-llm-registration/v1",
      id: "browser-llm",
      current_model_uid: 1,
      providers: [
        {
          id: 1,
          name: "Host model proxy",
          type: "openai",
          base_url: baseUrl,
          api_key: apiKey,
          api_paradigm: "openai_chat_completions",
          enabled_models: [{ uid: 1, model_id: modelId }],
        },
      ],
    },
    clusters: [
      {
        schema: "agent-runtime-agent-cluster-registration/v1",
        id: "browser-team",
        focus_agent_id: "main",
        agents: [{ id: "main", role: "browser-assistant", model_uid: 1 }],
      },
    ],
    endpoints: [{ id: "browser-tools", protocol: "host", timeout_ms: 10000 }],
    async host(operation, payload, { signal }) {
      signal.throwIfAborted();
      if (operation === "rpc.list_tools")
        return {
          schema: "corework-agent-tool/v1",
          tools: [
            {
              name: "WebEcho",
              description: "返回用户提供的文字。",
              readonly: true,
              idempotent: true,
              parameters: [
                { name: "value", param_type: "String", required: true },
              ],
              outputs: [{ name: "value", field_type: "String" }],
            },
          ],
        };
      if (
        operation === "rpc.execute" &&
        payload.request.tool_name === "WebEcho"
      ) {
        return {
          result: { value: payload.request.args_json.value },
          to_ai: "已返回文字。",
          error_code: 0,
        };
      }
      throw new Error(`Unsupported host operation: ${operation}`);
    },
  });
  const unsubscribe = runtime.subscribe(onEvent);
  try {
    const { conversation_id } = await runtime.invoke("conversation.spawn", {
      cluster_id: "browser-team",
    });
    return {
      runtime,
      conversation_id,
      send: (content) =>
        runtime.invoke("conversation.send_message", {
          conversation_id,
          content,
        }),
      pause: () => runtime.invoke("conversation.pause", { conversation_id }),
      async close() {
        try {
          await runtime.invoke("conversation.close", { conversation_id });
        } finally {
          unsubscribe();
          await runtime.dispose();
        }
      },
    };
  } catch (error) {
    unsubscribe();
    await runtime.dispose();
    throw error;
  }
}
