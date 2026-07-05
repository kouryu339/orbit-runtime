import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('.', import.meta.url));
const outDir = fileURLToPath(new URL('../playground-dist', import.meta.url));

export default defineConfig({
  root,
  base: './',
  build: {
    outDir,
    emptyOutDir: true,
  },
});
