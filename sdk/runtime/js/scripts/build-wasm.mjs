import { spawnSync } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
const root = fileURLToPath(new URL("../../../../", import.meta.url));
const target = process.env.CARGO_TARGET_DIR
  ? resolve(root, process.env.CARGO_TARGET_DIR)
  : resolve(root, "target");
const output = fileURLToPath(new URL("../dist/wasm/", import.meta.url));
const bindgen = process.env.WASM_BINDGEN ?? "wasm-bindgen";
function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    windowsHide: true,
    env: { ...process.env, CARGO_INCREMENTAL: "0" },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw Error(`${command} failed (${result.status})`);
}
const version = spawnSync(bindgen, ["--version"], {
  encoding: "utf8",
  windowsHide: true,
});
if (!version.stdout?.includes("0.2.128"))
  throw Error(
    "wasm-bindgen-cli 0.2.128 is required; set WASM_BINDGEN to its executable",
  );
run("cargo", [
  "build",
  "-p",
  "orbit-runtime-wasm",
  "--release",
  "--locked",
  "--target",
  "wasm32-unknown-unknown",
]);
await mkdir(output, { recursive: true });
run(bindgen, [
  "--target",
  "web",
  "--remove-name-section",
  "--remove-producers-section",
  "--out-dir",
  output,
  resolve(target, "wasm32-unknown-unknown/release/orbit_runtime_wasm.wasm"),
]);
await import("./build.mjs");
