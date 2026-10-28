import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    conditions: ['browser'],
    alias: Object.fromEntries(
      ['host', 'protocol', 'content', 'transport'].map((name) => [
        `@agent-runtime/conversation-element/${name}`,
        fileURLToPath(new URL(`../lit/src/${name}/index.ts`, import.meta.url)),
      ]),
    ),
  },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.ts'],
    setupFiles: ['./tests/setup.ts'],
    server: { deps: { inline: [/conversation-element/, /dompurify/] } },
  },
});
