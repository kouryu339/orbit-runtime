import { fileURLToPath } from "node:url";
export default {
  root: fileURLToPath(new URL("./bundler/", import.meta.url)),
  base: "./",
  worker: { format: "es" },
  build: {
    outDir: fileURLToPath(
      new URL("../../../../target/wasm-bundler/", import.meta.url),
    ),
    emptyOutDir: false,
    assetsInlineLimit: 0,
  },
};
