if (!document.doctype) {
  document.insertBefore(
    document.implementation.createDocumentType('html', '', ''),
    document.documentElement,
  );
}
(
  globalThis as typeof globalThis & { litDisableBundleWarning?: boolean }
).litDisableBundleWarning = true;
