import { createServer } from "node:http";
import { readFile, mkdtemp } from "node:fs/promises";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
const root = fileURLToPath(new URL("../", import.meta.url));
const WebSocket = createRequire(
  new URL("../../../../ui/lit/package.json", import.meta.url),
)("ws");
const profile = await mkdtemp(
  fileURLToPath(new URL("../../../../target/wasm-chrome-", import.meta.url)),
);
const server = createServer(async (req, res) => {
  if (req.url?.startsWith("/bundled/")) {
    try {
      const bundleRoot = fileURLToPath(
        new URL("../../../../target/wasm-bundler/", import.meta.url),
      );
      const file = path.resolve(
        bundleRoot,
        "." + decodeURIComponent(req.url.slice("/bundled".length)),
      );
      if (!file.startsWith(bundleRoot)) throw Error("invalid bundle path");
      const data = await readFile(file);
      res.setHeader(
        "Content-Type",
        file.endsWith(".wasm")
          ? "application/wasm"
          : file.endsWith(".html")
            ? "text/html; charset=utf-8"
            : "text/javascript; charset=utf-8",
      );
      res.end(data);
    } catch (error) {
      res.writeHead(404);
      res.end();
    }
    return;
  }
  if (req.url === "/v1/chat/completions") {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const input = JSON.parse(Buffer.concat(chunks));
    const output = input.messages.some((message) => message.role === "tool")
      ? { content: "Browser agent finished." }
      : {
          tool_calls: [
            {
              index: 0,
              id: "browser_call_1",
              type: "function",
              function: { name: "WebEcho", arguments: '{"value":"browser"}' },
            },
          ],
        };
    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
    });
    res.write(
      `data: ${JSON.stringify({ id: "test-chat", choices: [{ index: 0, delta: output, finish_reason: null }] })}\n\n`,
    );
    setTimeout(
      () =>
        res.end(
          `data: ${JSON.stringify({ id: "test-chat", choices: [{ index: 0, delta: {}, finish_reason: output.tool_calls ? "tool_calls" : "stop" }] })}\n\ndata: [DONE]\n\n`,
        ),
      20,
    );
    return;
  }
  try {
    const file = path.resolve(
      root,
      "." + decodeURIComponent(new URL(req.url, "http://localhost").pathname),
    );
    if (!file.startsWith(root)) throw Error("invalid path");
    const body = await readFile(file);
    res.setHeader(
      "Content-Type",
      file.endsWith(".wasm")
        ? "application/wasm"
        : file.endsWith(".html")
          ? "text/html; charset=utf-8"
          : "text/javascript; charset=utf-8",
    );
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end();
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const browser = spawn(
  process.env.CHROME_PATH ??
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
  [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "about:blank",
  ],
  { windowsHide: true, stdio: ["ignore", "ignore", "pipe"] },
);
let endpoint = "";
browser.stderr.on("data", (data) => {
  endpoint += data.toString();
});
let socket;
try {
  for (
    let n = 0;
    n < 100 && !/DevTools listening on (ws:\/\/[^\s]+)/.test(endpoint);
    n++
  )
    await new Promise((resolve) => setTimeout(resolve, 100));
  const url = endpoint.match(/DevTools listening on (ws:\/\/[^\s]+)/)?.[1];
  if (!url) throw Error("Chrome did not start: " + endpoint);
  const port = new URL(url).port;
  const tabs = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = tabs.find(
    (tab) => tab.type === "page" && tab.url === "about:blank",
  );
  if (!page) throw Error("Isolated browser page missing");
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = (error) => reject(Error(String(error.message)));
    setTimeout(
      () => reject(Error("Chrome handshake timed out")),
      10000,
    ).unref();
  });
  let seq = 0;
  const pending = new Map();
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    if (message.id) {
      const item = pending.get(message.id);
      pending.delete(message.id);
      message.error
        ? item?.reject(Error(JSON.stringify(message.error)))
        : item?.resolve(message.result);
    } else if (message.method === "Runtime.exceptionThrown")
      console.error(JSON.stringify(message.params));
  };
  const call = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++seq;
      pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });
  await call("Page.enable");
  await call("Runtime.enable");
  await call("Page.navigate", {
    url: `http://127.0.0.1:${server.address().port}${process.env.TEST_PAGE ?? "/tests/browser.html"}`,
  });
  let result;
  for (let n = 0; n < 150; n++) {
    result = await call("Runtime.evaluate", {
      expression: "window.verification",
      awaitPromise: true,
      returnByValue: true,
      timeout: 120000,
    });
    if (result.result?.value) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  if (!result.result?.value)
    console.error(
      JSON.stringify(
        await call("Runtime.evaluate", {
          expression: "({url:location.href,body:document.body.innerHTML})",
          returnByValue: true,
        }),
      ),
    );
  console.log(JSON.stringify(result.result?.value ?? result, null, 2));
  if (process.env.VERIFICATION_OUT) {
    const { writeFile } = await import("node:fs/promises");
    await writeFile(
      process.env.VERIFICATION_OUT,
      JSON.stringify(result.result?.value ?? result, null, 2) + "\n",
    );
  }
  if (!result.result?.value?.ok) process.exitCode = 1;
} finally {
  socket?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}
