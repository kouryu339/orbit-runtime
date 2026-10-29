import { mkdir, cp, readFile, writeFile, stat } from "node:fs/promises";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
let sourceCommit = null;
try {
  sourceCommit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: new URL("../../../../", import.meta.url),
    encoding: "utf8",
    windowsHide: true,
  }).trim();
} catch {
  /* Standalone SDK source archives need not contain Git metadata. */
}
await mkdir(new URL("../dist", import.meta.url), { recursive: true });
for (const name of ["index.js", "worker.js", "storage.js", "index.d.ts"])
  await cp(
    new URL(`../src/${name}`, import.meta.url),
    new URL(`../dist/${name}`, import.meta.url),
  );
for (const name of ["LICENSE", "NOTICE"]) {
  await cp(
    new URL(`../../../../${name}`, import.meta.url),
    new URL(`../${name}`, import.meta.url),
  );
  await cp(
    new URL(`../../../../${name}`, import.meta.url),
    new URL(`../dist/${name}`, import.meta.url),
  );
}

await stat(new URL("../dist/wasm/orbit_runtime_wasm_bg.wasm", import.meta.url));
await mkdir(new URL("../dist/schemas/", import.meta.url), { recursive: true });
for (const name of ["logic.schema.json", "resource.schema.json"])
  await cp(
    new URL(`../../../../corework/src/workflow/${name}`, import.meta.url),
    new URL(`../dist/schemas/${name}`, import.meta.url),
  );
const metadata = JSON.parse(
  await readFile(new URL("../package.json", import.meta.url), "utf8"),
);
const files = {};
for (const name of [
  "index.js",
  "index.d.ts",
  "worker.js",
  "storage.js",
  "wasm/orbit_runtime_wasm.js",
  "wasm/orbit_runtime_wasm.d.ts",
  "wasm/orbit_runtime_wasm_bg.wasm",
  "wasm/orbit_runtime_wasm_bg.wasm.d.ts",
  "schemas/logic.schema.json",
  "schemas/resource.schema.json",
  "LICENSE",
  "NOTICE",
]) {
  const bytes = await readFile(new URL(`../dist/${name}`, import.meta.url));
  files[name] = {
    bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}
await writeFile(
  new URL("../dist/manifest.json", import.meta.url),
  JSON.stringify(
    {
      schema: "orbit-runtime-browser-package/v1",
      version: metadata.version,
      source_commit: sourceCommit,
      target: "wasm32-unknown-unknown",
      worker: "worker.js",
      entry: "index.js",
      types: "index.d.ts",
      wasm_bindgen: "0.2.128",
      commands: "agent-runtime-command/v1",
      events: "agent-runtime-event/v1",
      files,
    },
    null,
    2,
  ) + "\n",
);
