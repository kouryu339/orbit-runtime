<script lang="ts">
  export let name: string;
  export let sizeBytes: number | undefined = undefined;
  export let locale = 'en-US';
  export let onRemove: (() => void) | undefined = undefined;
  export let disabled = false;

  const fileKinds = new Map([
    ['pdf', 'PDF'],
    ['doc', 'Word'], ['docx', 'Word'],
    ['xls', 'Excel'], ['xlsx', 'Excel'],
    ['csv', 'CSV'],
    ['ppt', 'PowerPoint'], ['pptx', 'PowerPoint'],
    ['txt', 'TXT'], ['md', 'Markdown'],
  ]);
  $: zh = locale.toLowerCase().startsWith('zh');
  $: kind = fileKinds.get(name.match(/\.([a-z0-9]+)$/i)?.[1].toLowerCase() ?? '') ??
    (zh ? '文件' : 'File');
  $: size = fileSize(sizeBytes);

  function fileSize(bytes: number | undefined) {
    if (typeof bytes !== 'number' || !Number.isSafeInteger(bytes) || bytes < 0)
      return '';
    const unit =
      bytes >= 1024 ** 3 ? 'GB' : bytes >= 1024 ** 2 ? 'MB' : bytes >= 1024 ? 'KB' : 'B';
    const divisor = { B: 1, KB: 1024, MB: 1024 ** 2, GB: 1024 ** 3 }[unit];
    const size = (bytes / divisor).toLocaleString(undefined, { maximumFractionDigits: 1 });
    return `${size} ${unit}`;
  }
</script>

<li class="file-attachment">
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
    <path d="M6 3h8l4 4v14H6z M14 3v5h4 M9 12h6 M9 16h6" />
  </svg>
  <div class="file-details">
    <span class="file-name">{name}</span>
    <small>{kind}{size ? ` · ${size}` : ''}</small>
  </div>
  {#if onRemove}
    <button type="button" aria-label={zh ? `移除 ${name}` : `Remove ${name}`} {disabled} on:click={onRemove}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
        <path d="m7 7 10 10M17 7 7 17" />
      </svg>
    </button>
  {/if}
</li>

<style>
  .file-attachment {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    max-width: 100%;
    padding: 5px 8px;
    background: var(--conversation-code-background);
    border-radius: 6px;
  }
  svg { width: 18px; height: 18px; flex: none; }
  .file-details { min-width: 0; white-space: normal; }
  .file-name { display: block; overflow-wrap: anywhere; }
  small { display: block; }
  button {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  button:hover:not(:disabled) { background: var(--conversation-surface-raised); }
  button:disabled { opacity: 0.5; cursor: default; }
</style>
