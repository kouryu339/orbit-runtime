import { defineConfig, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';

/** Ship one importable JS, including component styles and embedded KaTeX fonts. */
function standaloneStyles(): Plugin {
  return {
    name: 'orbit-svelte-standalone-styles',
    enforce: 'post',
    generateBundle: {
      order: 'post',
      handler(_options, bundle) {
        const css = Object.values(bundle).filter(
          (asset) => asset.type === 'asset' && asset.fileName.endsWith('.css'),
        );
        const text = css
          .map((asset) => (asset.type === 'asset' ? String(asset.source) : ''))
          .join('\n');
        const entry = Object.values(bundle).find(
          (asset) => asset.type === 'chunk' && asset.isEntry,
        );
        if (!entry || entry.type !== 'chunk' || !text)
          throw new Error('Standalone frontend entry or styles are missing.');
        entry.code = `const orbitStyleId="orbit-conversation-svelte-styles";if(typeof document!=="undefined"&&!document.getElementById(orbitStyleId)){const style=document.createElement("style");style.id=orbitStyleId;style.textContent=${JSON.stringify(text)};document.head.appendChild(style);}\n${entry.code}`;
        for (const asset of css) delete bundle[asset.fileName];
      },
    },
  };
}

function thirdPartyNotices(): Plugin {
  return {
    name: 'orbit-svelte-third-party-notices',
    generateBundle() {
      const packages = new Map<string, string>();
      for (const moduleId of this.getModuleIds()) {
        if (!moduleId.includes('node_modules') || moduleId.startsWith('\0'))
          continue;
        let directory = path.dirname(moduleId.split('?')[0]);
        while (directory !== path.dirname(directory)) {
          const manifest = path.join(directory, 'package.json');
          if (existsSync(manifest)) {
            const metadata = JSON.parse(readFileSync(manifest, 'utf8'));
            if (metadata.name && metadata.version) {
              const key = `${metadata.name}@${metadata.version}`;
              if (!packages.has(key)) {
                const licenses = readdirSync(directory, { withFileTypes: true })
                  .filter(
                    (entry) =>
                      entry.isFile() &&
                      /^(licen[cs]e|copying|notice)(\.|$)/i.test(entry.name),
                  )
                  .map((entry) => entry.name);
                packages.set(
                  key,
                  [
                    key,
                    `License: ${typeof metadata.license === 'string' ? metadata.license : JSON.stringify(metadata.license ?? 'See package source')}`,
                    ...licenses.map(
                      (name) =>
                        `${name}\n${readFileSync(path.join(directory, name), 'utf8')}`,
                    ),
                  ].join('\n\n'),
                );
              }
              break;
            }
          }
          directory = path.dirname(directory);
        }
      }
      this.emitFile({
        type: 'asset',
        fileName: 'THIRD_PARTY_NOTICES.txt',
        source: [...packages]
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([, notice]) => notice)
          .join('\n\n--------------------\n\n'),
      });
    },
  };
}

export default defineConfig({
  plugins: [svelte(), standaloneStyles(), thirdPartyNotices()],
  build: {
    outDir: 'dist-library',
    emptyOutDir: true,
    target: 'es2022',
    minify: 'esbuild',
    sourcemap: false,
    lib: {
      entry: fileURLToPath(new URL('./src/browser.ts', import.meta.url)),
      formats: ['es'],
      fileName: () => 'orbit-conversation-svelte.js',
    },
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
});
