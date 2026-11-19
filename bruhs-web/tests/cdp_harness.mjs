// Shared by the CDP browser harnesses (today_replay_browser.mjs,
// workflow_pause_browser.mjs, inbox_states_browser.mjs,
// people_profile_browser.mjs): a static server for
// a built dist, headless Chrome on a fixed debugging port with a fresh
// profile, and a flat-session CDP client with real pointer and key input.
//
// Nothing leaves the machine and nothing reaches a bruhs-server: every request
// that is not for the dist is refused. Loopback /api requests fail as a
// stopped server would, unless the page's fixture `api` answers them;
// anything else is recorded as unexpected.
//
// Each page gets inert IndexedDB hooks, switched on per check through
// `window.__qa` (see `QA_HOOKS`): delay a read-write transaction's
// completion event (the commit has happened, the app learns of it later),
// refuse writes with QuotaExceededError, and delay a database open's success.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createReadStream } from 'node:fs';
import { mkdir, rm, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';

export const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export const CHROME = process.env.BRUHS_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

const CONTENT_TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.mjs': 'text/javascript',
  '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2',
  '.json': 'application/json', '.png': 'image/png', '.ico': 'image/x-icon', '.txt': 'text/plain',
};

/** Serves `dist` on 127.0.0.1:`port`. `/__qa_blank` is an empty same-origin page. */
export async function serveDist(dist, port) {
  const root = path.resolve(dist);
  await stat(path.join(root, 'index.html'));
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    if (!['GET', 'HEAD'].includes(request.method)) { response.writeHead(405); response.end(); return; }
    if (url.pathname === '/__qa_stream') {
      // An event stream that stays open and says nothing, for fixture APIs.
      response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-store', 'Access-Control-Allow-Origin': '*' });
      response.write(': fixture\n\n');
      const timer = setInterval(() => response.write(': connected\n\n'), 1000);
      request.on('close', () => clearInterval(timer));
      return;
    }
    if (url.pathname === '/__qa_blank') {
      response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' });
      response.end('<!doctype html><title>QA fixture page</title><p>Same-origin IndexedDB fixture page.</p>');
      return;
    }
    const file = path.resolve(root, `.${url.pathname === '/' ? '/index.html' : decodeURIComponent(url.pathname)}`);
    if (!file.startsWith(root + path.sep)) { response.writeHead(403); response.end(); return; }
    try {
      const info = await stat(file);
      if (!info.isFile()) throw new Error('not a file');
      // Hashed assets may be cached, so a reload does not refetch the WASM.
      const cache = path.basename(file) === 'index.html' ? 'no-store' : 'max-age=3600';
      response.writeHead(200, { 'Content-Type': CONTENT_TYPES[path.extname(file)] ?? 'application/octet-stream', 'Content-Length': info.size, 'Cache-Control': cache });
      if (request.method === 'HEAD') { response.end(); return; }
      createReadStream(file).pipe(response);
    } catch { response.writeHead(404); response.end(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  return { origin: `http://127.0.0.1:${port}`, close: () => new Promise(resolve => server.close(resolve)) };
}

class Connection {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Set();
    ws.addEventListener('message', event => {
      const message = JSON.parse(event.data);
      if (message.id !== undefined) {
        const waiting = this.pending.get(message.id);
        if (!waiting) return;
        this.pending.delete(message.id);
        if (message.error) waiting.reject(new Error(`${waiting.method}: ${message.error.message}`));
        else waiting.resolve(message.result);
        return;
      }
      for (const listener of this.listeners) listener(message);
    });
  }
  send(method, params = {}, sessionId = undefined) {
    const id = this.nextId++;
    const payload = { id, method, params, ...(sessionId ? { sessionId } : {}) };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method });
      this.ws.send(JSON.stringify(payload));
    });
  }
}

/** Headless Chrome with a fresh profile under `profile`, debugging on `port`. */
export async function launchChrome({ port, profile }) {
  await rm(profile, { recursive: true, force: true });
  await mkdir(profile, { recursive: true });
  const child = spawn(CHROME, [
    '--headless=new', `--remote-debugging-port=${port}`, '--remote-debugging-address=127.0.0.1',
    `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--disable-extensions',
    '--disable-background-networking', '--disable-component-update', '--disable-sync', '--disable-default-apps',
    '--no-pings', '--hide-scrollbars', '--mute-audio', '--disable-features=Translate,MediaRouter,OptimizationHints',
    'about:blank',
  ], { stdio: 'ignore' });
  let version = null;
  for (let attempt = 0; attempt < 150 && !version; attempt++) {
    try { version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json(); } catch { await sleep(200); }
  }
  assert(version, `Chrome did not open its debugging port ${port}`);
  const ws = new WebSocket(version.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.addEventListener('open', resolve, { once: true }); ws.addEventListener('error', reject, { once: true }); });
  const browser = new Connection(ws);
  return {
    browser,
    version: version.Browser,
    async close() {
      try { await browser.send('Browser.close'); } catch {}
      ws.close();
      const exited = new Promise(resolve => child.once('exit', resolve));
      if (child.exitCode === null && child.signalCode === null) {
        if (await Promise.race([exited.then(() => true), sleep(5000).then(() => false)]) === false) child.kill('SIGKILL');
      }
    },
  };
}

/** Installed before any page script; inert until `window.__qa` asks. */
export const QA_HOOKS = `(() => {
  if (window.__qaHooks) return;
  window.__qaHooks = true;
  // Settings made with Page.qa() survive a reload of the same tab.
  let saved = {};
  try { saved = JSON.parse(sessionStorage.getItem('__qa') || '{}'); } catch {}
  const qa = window.__qa = Object.assign({ commitDelayMs: 0, refuseWrites: false, openDelayMs: 0, delayedCommits: 0, refusedWrites: 0, delayedOpens: 0 }, saved, window.__qa || {});
  const completion = Object.getOwnPropertyDescriptor(IDBTransaction.prototype, 'oncomplete');
  Object.defineProperty(IDBTransaction.prototype, 'oncomplete', {
    configurable: true, enumerable: true,
    get() { return completion.get.call(this); },
    set(handler) {
      if (typeof handler !== 'function' || this.mode === 'readonly') return completion.set.call(this, handler);
      const transaction = this;
      completion.set.call(this, function (event) {
        const delay = qa.commitDelayMs;
        if (!(delay > 0)) return handler.call(transaction, event);
        qa.delayedCommits += 1;
        setTimeout(() => handler.call(transaction, event), delay);
      });
    },
  });
  for (const method of ['put', 'add', 'delete', 'clear']) {
    const original = IDBObjectStore.prototype[method];
    IDBObjectStore.prototype[method] = function (...args) {
      if (qa.refuseWrites) { qa.refusedWrites += 1; throw new DOMException('The quota has been exceeded.', 'QuotaExceededError'); }
      return original.apply(this, args);
    };
  }
  const opened = Object.getOwnPropertyDescriptor(IDBRequest.prototype, 'onsuccess');
  Object.defineProperty(IDBOpenDBRequest.prototype, 'onsuccess', {
    configurable: true, enumerable: true,
    get() { return opened.get.call(this); },
    set(handler) {
      if (typeof handler !== 'function') return opened.set.call(this, handler);
      const request = this;
      opened.set.call(this, function (event) {
        const delay = qa.openDelayMs;
        if (!(delay > 0)) return handler.call(request, event);
        qa.delayedOpens += 1;
        setTimeout(() => handler.call(request, event), delay);
      });
    },
  });
})();`;

const KEYS = {
  Tab: { key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9 },
  Enter: { key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r' },
  Escape: { key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 },
  Space: { key: ' ', code: 'Space', windowsVirtualKeyCode: 32, text: ' ' },
  ArrowDown: { key: 'ArrowDown', code: 'ArrowDown', windowsVirtualKeyCode: 40 },
  ArrowUp: { key: 'ArrowUp', code: 'ArrowUp', windowsVirtualKeyCode: 38 },
};
const loopback = hostname => ['127.0.0.1', 'localhost', '[::1]', '::1'].includes(hostname);

export class Page {
  constructor(browser, sessionId, targetId, origin) {
    Object.assign(this, { browser, sessionId, targetId, origin });
    this.refusedApi = [];
    this.unexpected = [];
    this.runtimeErrors = [];
    this.consoleErrors = [];
    this.handlers = new Set();
    browser.listeners.add(message => {
      if (message.sessionId !== sessionId) return;
      if (message.method === 'Fetch.requestPaused') { this.#route(message.params); return; }
      if (message.method === 'Runtime.exceptionThrown') this.runtimeErrors.push(message.params.exceptionDetails?.exception?.description ?? message.params.exceptionDetails?.text);
      if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') this.consoleErrors.push(message.params.args.map(arg => arg.value ?? arg.description ?? '').join(' ').slice(0, 500));
      for (const handler of this.handlers) handler(message);
    });
  }

  send(method, params = {}) { return this.browser.send(method, params, this.sessionId); }

  async #route({ requestId, request }) {
    const url = new URL(request.url);
    try {
      if (url.origin === this.origin) { await this.send('Fetch.continueRequest', { requestId }); return; }
      if (loopback(url.hostname) && url.pathname.startsWith('/api/')) {
        const answer = this.api && (request.method === 'OPTIONS' ? { status: 204, body: null } : await this.api({ method: request.method, url, body: request.postData ?? null }));
        if (answer === 'stream') {
          await this.send('Fetch.continueRequest', { requestId, url: `${this.origin}/__qa_stream` });
          return;
        }
        if (answer) {
          const responseHeaders = [
            ['Access-Control-Allow-Origin', this.origin], ['Access-Control-Allow-Headers', '*'],
            ['Access-Control-Allow-Methods', 'GET, HEAD, OPTIONS, POST, PATCH, PUT, DELETE'],
            ['Access-Control-Allow-Private-Network', 'true'], ['Cache-Control', 'no-store'], ['Content-Type', 'application/json'],
          ].map(([name, value]) => ({ name, value }));
          const body = answer.body === null ? '' : JSON.stringify(answer.body);
          await this.send('Fetch.fulfillRequest', { requestId, responseCode: answer.status ?? 200, responseHeaders, body: Buffer.from(body).toString('base64') });
          return;
        }
        this.refusedApi.push(`${request.method} ${url.pathname}`);
        await this.send('Fetch.failRequest', { requestId, errorReason: 'ConnectionRefused' });
        return;
      }
      this.unexpected.push(`${request.method} ${request.url}`);
      await this.send('Fetch.failRequest', { requestId, errorReason: 'BlockedByClient' });
    } catch {}
  }

  /**
   * `api`, if given, answers loopback /api requests: it gets
   * `{ method, url, body }` and returns `{ status, body }` (JSON), the string
   * 'stream' for a quiet event stream, or nothing to refuse the request; or
   * a promise of one, to answer late (or, never settling, not at all).
   */
  async init({ width = 1280, height = 800, touch = false, initScripts = [], timezone = undefined, api = undefined } = {}) {
    this.api = api;
    await this.send('Page.enable');
    await this.send('Runtime.enable');
    await this.send('Fetch.enable', { patterns: [{ urlPattern: '*', requestStage: 'Request' }] });
    await this.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: touch });
    if (timezone) await this.send('Emulation.setTimezoneOverride', { timezoneId: timezone });
    if (touch) await this.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
    await this.send('Emulation.setFocusEmulationEnabled', { enabled: true });
    await this.send('Page.addScriptToEvaluateOnNewDocument', { source: QA_HOOKS });
    for (const source of initScripts) await this.send('Page.addScriptToEvaluateOnNewDocument', { source });
    this.size = { width, height };
    return this;
  }

  async addInitScript(source) {
    const { identifier } = await this.send('Page.addScriptToEvaluateOnNewDocument', { source });
    return identifier;
  }

  removeInitScript(identifier) { return this.send('Page.removeScriptToEvaluateOnNewDocument', { identifier }); }

  async evaluate(expression) {
    const result = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(`evaluate: ${(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text ?? '').slice(0, 600)}`);
    return result.result.value;
  }

  async waitFor(expression, timeout = 20_000, interval = 100) {
    const deadline = Date.now() + timeout;
    let last = null;
    while (Date.now() < deadline) {
      try { if (await this.evaluate(expression)) return true; } catch (error) { last = error; }
      await sleep(interval);
    }
    if (last) this.lastWaitError = String(last);
    return false;
  }

  async goto(url) { await this.send('Page.navigate', { url }); }

  /** Reloads and waits for the new document (a navigation right after
   * Page.reload would cancel it and keep the old one). */
  async reload(timeout = 60_000) {
    const marker = `${Date.now()}-${Math.random()}`;
    await this.evaluate(`window.__qaDocument = ${JSON.stringify(marker)}`);
    await this.send('Page.reload', { ignoreCache: false });
    const fresh = await this.waitFor(`window.__qaDocument !== ${JSON.stringify(marker)} && document.readyState === 'complete'`, timeout);
    assert(fresh, 'the page reloaded');
  }

  /** Switches the IndexedDB hooks for this tab, across its reloads. */
  async qa(settings) {
    await this.evaluate(`(() => {
      Object.assign(window.__qa, ${JSON.stringify(settings)});
      const { commitDelayMs, refuseWrites, openDelayMs } = window.__qa;
      try { sessionStorage.setItem('__qa', JSON.stringify({ commitDelayMs, refuseWrites, openDelayMs })); } catch {}
    })()`);
  }

  /** Waits for the app shell: the WASM has started and the sidebar rendered. */
  async appReady(timeout = 60_000) {
    return this.waitFor(`document.readyState === 'complete' && !!document.querySelector('main, [data-app-shell], nav')`, timeout);
  }

  /**
   * Centre of the element the expression returns, after scrolling it into
   * view; `hit` says whether a pointer there lands on it.
   */
  async locate(elementExpression) {
    return this.evaluate(`(() => {
      const element = (${elementExpression});
      if (!element) return null;
      element.scrollIntoView({ block: 'center', inline: 'center' });
      const rect = element.getBoundingClientRect();
      const x = rect.left + rect.width / 2, y = rect.top + rect.height / 2;
      const top = document.elementFromPoint(x, y);
      return { x, y, width: rect.width, height: rect.height, hit: !!top && (top === element || element.contains(top)) };
    })()`);
  }

  /** A real pointer click (move, press, release) at the element's centre. */
  async click(elementExpression) {
    const box = await this.locate(elementExpression);
    if (!box) return { clicked: false, reason: 'missing' };
    if (!box.hit) return { clicked: false, reason: 'covered', box };
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: box.x, y: box.y });
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: box.x, y: box.y, button: 'left', buttons: 1, clickCount: 1 });
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: box.x, y: box.y, button: 'left', buttons: 0, clickCount: 1 });
    return { clicked: true, box };
  }

  async press(name, modifiers = 0) {
    const key = KEYS[name];
    assert(key, `unknown key ${name}`);
    await this.send('Input.dispatchKeyEvent', { type: 'keyDown', modifiers, ...key, ...(key.text ? { unmodifiedText: key.text } : {}) });
    await this.send('Input.dispatchKeyEvent', { type: 'keyUp', modifiers, ...key, text: undefined });
  }

  async type(text) { await this.send('Input.insertText', { text }); }

  async screenshot(file) {
    const { data } = await this.send('Page.captureScreenshot', { format: 'png' });
    await writeFile(file, Buffer.from(data, 'base64'));
    return file;
  }

  /**
   * Role and accessible name of every node Chrome exposes, in tree order,
   * from `Accessibility.getFullAXTree`. Ignored, generic and empty text
   * nodes are left out (their children are kept); `depth` counts the kept
   * ancestors, and a few states are kept when set.
   */
  async accessibilityTree() {
    await this.send('Accessibility.enable');
    const { nodes } = await this.send('Accessibility.getFullAXTree');
    await this.send('Accessibility.disable');
    const byId = new Map(nodes.map(node => [node.nodeId, node]));
    const property = (node, name) => node.properties?.find(entry => entry.name === name)?.value?.value;
    const skip = node => node.ignored
      || ['none', 'generic', 'InlineTextBox', 'LineBreak'].includes(node.role?.value)
      || (node.role?.value === 'StaticText' && !String(node.name?.value ?? '').trim());
    const out = [];
    const walk = (node, depth) => {
      let below = depth;
      if (!skip(node)) {
        const entry = { role: node.role?.value ?? '', name: String(node.name?.value ?? '') };
        for (const name of ['focused', 'disabled', 'pressed', 'checked', 'expanded', 'modal', 'level', 'invalid']) {
          const value = property(node, name);
          if (value !== undefined && value !== false && value !== 'false') entry[name] = value;
        }
        entry.depth = depth;
        out.push(entry);
        below = depth + 1;
      }
      for (const id of node.childIds ?? []) {
        const child = byId.get(id);
        if (child) walk(child, below);
      }
    };
    for (const root of nodes.filter(node => !node.parentId || !byId.has(node.parentId))) walk(root, 0);
    return out;
  }

  /** The focused element: tag, role, accessible label and text. */
  focused() {
    return this.evaluate(`(() => {
      const el = document.activeElement;
      if (!el || el === document.body) return { tag: el ? 'body' : null };
      return { tag: el.tagName.toLowerCase(), role: el.getAttribute('role'), label: el.getAttribute('aria-label'), text: (el.textContent || '').trim().replace(/\\s+/g, ' ').slice(0, 80), focusVisible: el.matches(':focus-visible') };
    })()`);
  }

  /** Every record in an app store, read through a separate connection. */
  readStore(store) {
    return this.evaluate(`new Promise((resolve, reject) => {
      const open = indexedDB.open('bruhs');
      open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('the app has not created its database')); };
      open.onerror = () => reject(new Error(String(open.error)));
      open.onsuccess = () => {
        const db = open.result;
        if (!db.objectStoreNames.contains(${JSON.stringify(store)})) { db.close(); reject(new Error('no store ' + ${JSON.stringify(store)})); return; }
        const all = db.transaction(${JSON.stringify(store)}, 'readonly').objectStore(${JSON.stringify(store)}).getAll();
        all.onsuccess = () => { db.close(); resolve(all.result); };
        all.onerror = () => { db.close(); reject(new Error(String(all.error))); };
      };
    })`);
  }

  async close() { try { await this.browser.send('Target.closeTarget', { targetId: this.targetId }); } catch {} }
}

/** A fresh browser context: its own IndexedDB, cookies and cache. */
export async function newContext(browser) {
  const { browserContextId } = await browser.send('Target.createBrowserContext', { disposeOnDetach: true });
  return {
    id: browserContextId,
    dispose: () => browser.send('Target.disposeBrowserContext', { browserContextId }).catch(() => {}),
  };
}

/** A new tab; tabs in one context share one IndexedDB. */
export async function newPage(browser, origin, { context = undefined, ...options } = {}) {
  const { targetId } = await browser.send('Target.createTarget', { url: 'about:blank', ...(context ? { browserContextId: context.id } : {}) });
  const { sessionId } = await browser.send('Target.attachToTarget', { targetId, flatten: true });
  const page = new Page(browser, sessionId, targetId, origin);
  return page.init(options);
}

/** Refuses a dist older than any of `sources` (paths relative to `root`). */
export async function assertFreshDist(root, dist, sources) {
  const index = await stat(path.join(dist, 'index.html'));
  const times = await Promise.all(sources.map(async file => ({ file, modifiedMs: (await stat(path.join(root, file))).mtimeMs })));
  const newest = times.reduce((left, right) => (right.modifiedMs > left.modifiedMs ? right : left));
  assert(index.mtimeMs >= newest.modifiedMs, `Refuse a stale dist: ${newest.file} changed after ${dist} was built`);
  return { indexModifiedMs: index.mtimeMs, newestSource: newest };
}

/** The first node `matches` accepts and every node below it, in order. */
export function subtree(nodes, matches) {
  const start = nodes.findIndex(matches);
  if (start < 0) return [];
  let end = start + 1;
  while (end < nodes.length && nodes[end].depth > nodes[start].depth) end += 1;
  return nodes.slice(start, end);
}

/** Records checks; `report.json` lists them with the run's environment. */
export function checklist(name) {
  const checks = [];
  const check = (passed, label, evidence = undefined) => {
    const entry = { scenario: check.scenario, label, passed: Boolean(passed), ...(evidence === undefined ? {} : { evidence }) };
    checks.push(entry);
    console.log(`${entry.passed ? 'PASS' : 'FAIL'} [${check.scenario}] ${label}${entry.passed || evidence === undefined ? '' : ` ${JSON.stringify(evidence).slice(0, 400)}`}`);
    return entry.passed;
  };
  check.scenario = name;
  check.all = checks;
  check.summary = () => ({ passed: checks.filter(entry => entry.passed).length, failed: checks.filter(entry => !entry.passed).length });
  return check;
}
