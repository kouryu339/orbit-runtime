import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { JSDOM } from 'jsdom';

const directory = new URL('../dist-library/', import.meta.url);
const files = await readdir(directory);
assert.deepEqual(files.sort(), [
  'THIRD_PARTY_NOTICES.txt',
  'orbit-conversation-svelte.js',
]);
const code = await readFile(
  new URL('orbit-conversation-svelte.js', directory),
  'utf8',
);
assert.ok(
  !/from\s*["'](?:svelte|lit|@agent-runtime)/.test(code),
  'runtime must be bundled',
);
assert.ok(
  !code.includes('acceptance-fixture'),
  'no acceptance fixtures in release',
);
const dom = new JSDOM(
  '<!doctype html><html><head></head><body><main></main></body></html>',
  {
    url: 'http://localhost/',
    pretendToBeVisual: true,
  },
);
for (const key of [
  'window',
  'document',
  'navigator',
  'Node',
  'Element',
  'HTMLElement',
  'HTMLMediaElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'Text',
  'Comment',
  'CustomEvent',
  'Event',
  'MutationObserver',
  'DOMParser',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
]) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value:
      typeof dom.window[key] === 'function' &&
      [
        'getComputedStyle',
        'requestAnimationFrame',
        'cancelAnimationFrame',
      ].includes(key)
        ? dom.window[key].bind(dom.window)
        : dom.window[key],
  });
}
window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
HTMLElement.prototype.scrollTo = function () {};
const frontend = await import(
  new URL('orbit-conversation-svelte.js', directory)
);
assert.equal(typeof frontend.mountConversation, 'function');
let disconnected = 0;
let sends = 0;
const transport = {
  contract: frontend.TRANSPORT_CONTRACT,
  id: 'release-smoke',
  async connect(context, handlers) {
    handlers.connection('connected');
    handlers.event({
      type: 'conversation-created',
      conversationId: 'release-smoke',
    });
    handlers.event({
      type: 'state-snapshot',
      conversationId: 'release-smoke',
      payload: { conversation_state: 'waiting', ledger_records: [] },
    });
    return {
      disconnect() {
        disconnected++;
      },
    };
  },
  async send(request) {
    assert.equal(request.content, 'release smoke');
    sends++;
    return { accepted: true };
  },
  async pause() {
    return { accepted: true };
  },
  async close() {
    return { accepted: true };
  },
  async resolveToolPermission() {
    return { accepted: true };
  },
};
const target = document.querySelector('main');
const instance = frontend.mountConversation(target, {
  transport,
  locale: 'zh-CN',
  colorScheme: 'light',
});
await new Promise((resolve) => setTimeout(resolve, 30));
assert.ok(target.querySelector('textarea'), 'compiled component mounts');
assert.ok(
  document
    .querySelector('#orbit-conversation-svelte-styles')
    ?.textContent.includes('data:font/'),
  'fonts embedded',
);
assert.equal((await instance.api.send('release smoke')).accepted, true);
assert.equal(sends, 1);
await instance.destroy();
await instance.destroy();
assert.equal(disconnected, 1, 'transport disconnects exactly once');
assert.equal(target.querySelector('textarea'), null);
dom.window.close();
console.log(
  JSON.stringify({
    passed: true,
    entry: 'orbit-conversation-svelte.js',
    mount: true,
    send: true,
    destroy: true,
  }),
);
