/** One live runtime owns a namespace. A crashed tab releases its Web Lock. */
export async function acquireNamespace(namespace) {
  if (!globalThis.navigator?.locks)
    throw new Error(
      "Persistent storage requires Web Locks in a secure context",
    );
  let release;
  let acquired;
  const ready = new Promise((resolve, reject) => {
    acquired = { resolve, reject };
  });
  const lifetime = navigator.locks.request(
    `orbit-runtime:${namespace}`,
    { ifAvailable: true },
    async (lock) => {
      if (!lock) {
        acquired.reject(
          new Error(`Runtime storage is already in use: ${namespace}`),
        );
        return;
      }
      await new Promise((resolve) => {
        release = resolve;
        acquired.resolve();
      });
    },
  );
  lifetime.catch((error) => acquired.reject(error));
  await ready;
  return async () => {
    release();
    await lifetime;
  };
}

export function indexedDBStore(namespace) {
  let database;
  async function open() {
    if (database) return database;
    database = await new Promise((resolve, reject) => {
      const request = indexedDB.open("orbit-runtime-browser-v1", 1);
      request.onupgradeneeded = () =>
        request.result.createObjectStore("resources");
      request.onsuccess = () => {
        request.result.onversionchange = () => request.result.close();
        resolve(request.result);
      };
      request.onerror = () => reject(request.error);
      request.onblocked = () =>
        reject(new Error("Runtime database upgrade is blocked"));
    });
    return database;
  }
  return {
    async load() {
      const db = await open();
      return new Promise((resolve, reject) => {
        const tx = db.transaction("resources", "readonly");
        const request = tx.objectStore("resources").get(namespace);
        tx.oncomplete = () =>
          resolve(request.result ?? { revision: 0, snapshot: null });
        tx.onerror = tx.onabort = () => reject(tx.error ?? request.error);
      });
    },
    async save(snapshot, expectedRevision) {
      const db = await open();
      return new Promise((resolve, reject) => {
        const tx = db.transaction("resources", "readwrite");
        const store = tx.objectStore("resources");
        const request = store.get(namespace);
        let conflict;
        request.onsuccess = () => {
          if ((request.result?.revision ?? 0) !== expectedRevision) {
            conflict = new Error("Resource storage revision conflict");
            tx.abort();
            return;
          }
          store.put({ revision: expectedRevision + 1, snapshot }, namespace);
        };
        tx.oncomplete = () => resolve(expectedRevision + 1);
        tx.onerror = tx.onabort = () =>
          reject(
            conflict ?? tx.error ?? new Error("Resource transaction aborted"),
          );
      });
    },
    close() {
      database?.close();
      database = undefined;
    },
  };
}

/** Explicitly ephemeral; never selected as a fallback for failed persistence. */
export function memoryStore() {
  let revision = 0,
    snapshot = null;
  return {
    async load() {
      return { revision, snapshot: structuredClone(snapshot) };
    },
    async save(next, expected) {
      if (expected !== revision)
        throw new Error("Resource storage revision conflict");
      snapshot = structuredClone(next);
      return ++revision;
    },
    close() {},
  };
}
