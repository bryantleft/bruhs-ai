// Build the WASM app with trunk, then pass the fresh dist path to this script.
// Uses two real app tabs, a disposable browser profile and synthetic IndexedDB data.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { platform } from 'node:os';
import { extname, resolve, sep } from 'node:path';

assert(process.argv[2], 'Pass the path to a fresh Bruhs web dist');
const dist = resolve(process.argv[2]);
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH ?? 'playwright');
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm' };
const server = createServer(async (request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  if (pathname === '/fixture') {
    response.writeHead(200, { 'Content-Type': mime['.html'] }).end('<!doctype html><title>Fixture</title>');
    return;
  }
  const file = resolve(dist, '.' + (pathname === '/' ? '/index.html' : pathname));
  try {
    assert(file.startsWith(dist + sep));
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': mime[extname(file)] ?? 'application/octet-stream' }).end(bytes);
  } catch {
    response.writeHead(404).end();
  }
});

await new Promise(done => server.listen(0, '127.0.0.1', done));
const origin = `http://127.0.0.1:${server.address().port}`;
let browser;
try {
  browser = await chromium.launch({
    executablePath: process.env.BRUHS_CHROME ?? (platform() === 'darwin'
      ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined),
    args: ['--disable-renderer-backgrounding', '--disable-backgrounding-occluded-windows', '--disable-background-timer-throttling'],
  });
  const context = await browser.newContext();
  context.setDefaultTimeout(15_000);
  await context.route('**/*', route => {
    const url = new URL(route.request().url());
    if (url.origin === origin && !url.pathname.startsWith('/api/')) return route.continue();
    return route.fulfill({
      contentType: 'application/json',
      headers: { 'access-control-allow-origin': '*' },
      body: 'null',
    });
  });

  const a = await context.newPage();
  const b = await context.newPage();
  const fixture = await context.newPage();
  const errors = [];
  for (const page of [a, b]) page.on('pageerror', error => errors.push(String(error)));
  const url = origin + '/#/tasks';
  console.error('Opening the task app and fixture');
  await a.goto(url);
  await a.getByRole('button', { name: 'Board', exact: true }).waitFor({ timeout: 30_000 });
  await a.evaluate(async () => {
    const deadline = performance.now() + 15_000;
    while (!(await indexedDB.databases()).some(db => db.name === 'bruhs' && db.version >= 13)) {
      if (performance.now() >= deadline) throw new Error('The app did not initialize IndexedDB');
      await new Promise(resolve => setTimeout(resolve, 50));
    }
  });
  await fixture.goto(origin + '/fixture');

  const taskId = 'live-board-probe';
  const title = 'Live board probe';
  const statuses = ['Backlog', 'Todo', 'InProgress', 'Done'];
  const rows = Array.from({ length: 201 }, (_, index) => ({
    id: index === 0 ? taskId : `fixture-task-${index}`,
    title: index === 0 ? title : `Task ${index}`,
    status: statuses[index % statuses.length],
    priority: 'None',
    project_id: null,
    description: null,
    due_date: null,
    created_at: 1_000 + index,
    updated_at: 1_000 + index,
  }));

  const put = updates => fixture.evaluate(changes => {
    return new Promise((done, fail) => {
      const open = indexedDB.open('bruhs');
      open.onupgradeneeded = () => {
        open.transaction.abort();
        fail(new Error('The app must create the IndexedDB schema before seeding'));
      };
      open.onerror = () => fail(open.error);
      open.onblocked = () => fail(new Error('IndexedDB fixture open was blocked'));
      open.onsuccess = () => {
        const db = open.result;
        try {
          const tx = db.transaction('work_items', 'readwrite');
          tx.oncomplete = () => { db.close(); done(); };
          tx.onabort = () => { db.close(); fail(tx.error); };
          const store = tx.objectStore('work_items');
          for (const item of changes) store.put(item);
        } catch (error) {
          db.close();
          fail(error);
        }
      };
    });
  }, updates);

  await put(rows);
  const board = page => page.getByRole('region', { name: 'Task board columns' });
  const column = (page, index) => board(page).locator(':scope > div').nth(index);
  const card = (page, index, label = title) => column(page, index).getByText(label, { exact: true });
  const selectBoard = async page => {
    await page.getByRole('button', { name: 'Board', exact: true }).click();
    await card(page, 0).waitFor();
  };
  await Promise.all([a.reload(), b.goto(url)]);
  await Promise.all([selectBoard(a), selectBoard(b)]);
  console.error('Both tabs loaded the seeded board');

  // Probe DOM availability and one subsequent frame on each tab. All stamps
  // use timeOrigin + performance.now for cross-page subtraction.
  const observe = async (page, lane, name = title) => {
    await column(page, lane).evaluate((target, label) => {
      window.taskPaint = 0;
      const ready = () => Array.from(target.querySelectorAll('[draggable="true"]'))
        .some(card => card.querySelector('.line-clamp-2')?.textContent === label);
      const observer = new MutationObserver(() => {
        if (!ready()) return;
        observer.disconnect();
        requestAnimationFrame(() => requestAnimationFrame(() => {
          window.taskPaint = performance.timeOrigin + performance.now();
        }));
      });
      observer.observe(target, { childList: true, subtree: true });
    }, name);
  };

  await a.evaluate(id => {
    const put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put = function (value, ...args) {
      if (this.name === 'work_items' && value.id === id) {
        this.transaction.addEventListener('complete', () => {
          window.taskCommit = performance.timeOrigin + performance.now();
        }, { once: true });
      }
      return put.call(this, value, ...args);
    };
  }, taskId);
  await b.evaluate(() => {
    window.taskReads = { keys: [], snapshots: 0 };
    const get = IDBObjectStore.prototype.get;
    const getAll = IDBObjectStore.prototype.getAll;
    IDBObjectStore.prototype.get = function (key) {
      if (this.name === 'work_items') window.taskReads.keys.push(key);
      return get.call(this, key);
    };
    IDBObjectStore.prototype.getAll = function (...args) {
      if (this.name === 'work_items') window.taskReads.snapshots++;
      return getAll.call(this, ...args);
    };
  });

  const drop = (page, lane) => column(page, lane).evaluate((target, id) => {
    const dataTransfer = new DataTransfer();
    dataTransfer.setData('text/plain', id);
    const start = performance.timeOrigin + performance.now();
    target.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
    return start;
  }, taskId);

  const measurements = [];
  for (let index = 0; index < 30; index++) {
    const lane = index % 2 === 0 ? 1 : 0;
    await Promise.all([observe(a, lane), observe(b, lane)]);
    const start = await drop(a, lane);
    await Promise.all([
      a.waitForFunction(start => window.taskPaint >= start && window.taskCommit >= start, start),
      b.waitForFunction(start => window.taskPaint >= start, start),
    ]);
    const [local, peer, commit] = await Promise.all([
      a.evaluate(() => window.taskPaint),
      b.evaluate(() => window.taskPaint),
      a.evaluate(() => window.taskCommit),
    ]);
    assert(peer >= commit, 'Peer visibility precedes durable commit');
    measurements.push({ local: local - start, peer: peer - commit });
  }
  assert.deepEqual(await b.evaluate(() => window.taskReads), {
    keys: Array(measurements.length).fill(taskId),
    snapshots: 0,
  });
  console.error('All thirty moves reached both tabs');

  // The card stays in Backlog and keeps its focus when a peer changes its title.
  const renamed = 'Renamed without moving';
  await column(b, 0).locator('[draggable="true"]').filter({ hasText: title }).focus();
  await card(a, 0).click();
  await a.locator('#task-detail-title').fill(renamed);
  await card(a, 0, renamed).waitFor();
  await card(b, 0, renamed).waitFor();
  assert.equal(await b.evaluate(() => document.activeElement?.textContent?.trim()), renamed);

  await drop(b, 1);
  await card(a, 1, renamed).waitFor();

  // A suspended tab can catch up from IndexedDB when it is shown again,
  // including a change with no notification at all.
  await b.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })));
  await put([{ ...rows[0], status: 'InProgress', title: 'Committed elsewhere' }]);
  await b.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true })));
  await card(b, 2, 'Committed elsewhere').waitFor();

  // Tab A still holds the previous record. Its failed CAS preserves the
  // optimistic draft and the durable winner stays visible in tab B.
  await drop(a, 3);
  await a.getByRole('region', { name: 'Unsaved local changes' }).waitFor();
  await card(a, 3, renamed).waitFor();
  await card(b, 2, 'Committed elsewhere').waitFor();
  assert.equal(await card(b, 3).count(), 0);
  assert.deepEqual(errors, []);

  const p95 = key => {
    const values = measurements.map(sample => sample[key]).sort((a, b) => a - b);
    return Math.round(values[Math.ceil(values.length * 0.95) - 1] * 10) / 10;
  };
  console.log(JSON.stringify({
    boardRows: rows.length, samples: measurements.length, localFrameP95Ms: p95('local'),
    commitToPeerFrameP95Ms: p95('peer'), resume: 'pass', staleWrite: 'pass',
  }));
} finally {
  await browser?.close();
  await new Promise(done => server.close(done));
}
