// Preserve Lit's visual baseline while keeping Svelte's rendering independent.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const root = path.dirname(fileURLToPath(import.meta.url));
const targets = [
  ['conversation-element.ts', 'conversation', '.orbit-svelte-conversation'],
  ['rich-content.ts', 'rich-content', '.orbit-svelte-rich-content'],
  ['plan-card.ts', 'plan', '.orbit-svelte-plan'],
  ['mermaid-diagram.ts', 'mermaid', '.orbit-svelte-mermaid'],
  ['image-reference.ts', 'image-reference', '.orbit-svelte-image-reference'],
];
for (const [source, name, scope] of targets) {
  const text = readFileSync(
    path.join(root, 'lit/src/components', source),
    'utf8',
  ).replace(/\r\n/g, '\n');
  const css = text.match(/static styles = css`([\s\S]*?)`;/)?.[1];
  if (!css) throw new Error(`Missing styles: ${source}`);
  const output =
    `/* Generated from Lit ${source}; run node ui/sync-svelte-styles.mjs. */\n` +
    css
      .replace(/([^{}]+)\{/g, (match, selector) => {
        if (selector.trim().startsWith('@')) return match;
        return (
          selector
            .split(',')
            .map((s) => {
              let value = s.trim();
              // Keyframe steps do not select elements.
              if (/^(from|to|[\d.]+%)$/.test(value)) return value;
              // Lit's generic control rules stop at child shadow roots. Mirror
              // that boundary for Svelte's independent nested renderers.
              if(name==='conversation' && /^button\b/.test(value))value=value.replace(/^button/,"button:not(:where(.orbit-svelte-rich-content *, .orbit-svelte-plan *, .orbit-svelte-image-reference *))");
              return value.includes(':host')
                ? value
                    .replace(/:host\(([^)]+)\)/g, `${scope}$1`)
                    .replaceAll(':host', scope)
                : `${scope} ${value}`;
            })
            .join(',\n') + ' {'
        );
      })
      .replaceAll('[theme=', '[data-theme=')
      .replaceAll('[density=', '[data-density=')
      .replaceAll('[color-scheme=', '[data-color-scheme=')
      .trim() +
    '\n';
  const target = path.join(root, 'svelte/src/lib', `${name}.css`);
  if (process.argv.includes('--check')) {
    if (readFileSync(target, 'utf8').replace(/\r\n/g, '\n') !== output)
      throw new Error(`Styles out of sync: ${target}`);
  } else writeFileSync(target, output);
}
