(globalThis as typeof globalThis & { litDisableBundleWarning?: boolean })
  .litDisableBundleWarning = true;

const originalWarn = console.warn.bind(console);
console.warn = (...args: unknown[]) => {
  const message = args.map(String).join(' ');
  if (
    message.includes('Lit is in dev mode') ||
    message.includes("KaTeX doesn't work in quirks mode")
  ) return;
  originalWarn(...args);
};

if (typeof document !== 'undefined' && !document.doctype) {
  document.insertBefore(
    document.implementation.createDocumentType('html', '', ''),
    document.documentElement,
  );
}
