// Acceptance run for the crew on the web (docs/crew.md; PARITY-16 in
// docs/web-desktop-parity-goal-2026-11-15.md): the rail, ⌘⇧B, a bruh's page
// and switching between pages, Settings (a face edit keeps a pending run, an
// access change cancels it), ⌘. and Resume all, reduced motion, phone-size
// touch targets and the create dialog's keyboard and Cancel behaviour. Any
// panic in the page fails the run. Every check prints ok or FAIL; the exit
// status is 0 only when all pass.
//
//   cargo build -p bruhs-server
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43031 trunk build --dist "$TMPDIR/bruhs-crew-dist")
//   node scripts/verify-crew.mjs "$PWD" "$TMPDIR/bruhs-crew-dist" 43031
//
// The bundle must be built for the port the check uses. bruhs-server runs
// with a disposable HOME and BRUHS_REACTIVE_DB, a capture URL nothing listens
// on and, first on its PATH, a stand-in `claude` that streams one line and
// waits to be stopped, so no provider is called and no real data is touched.
// Headless Chrome (BRUHS_VERIFY_CHROME, default the macOS app) runs on
// BRUHS_VERIFY_CDP_PORT (default 9431) with its own profile. Screenshots,
// logs and results go to $TMPDIR/bruhs-crew-verify/, cleared on each run.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const root = path.resolve(process.argv[2] ?? '');
const dist = path.resolve(process.argv[3] ?? '');
const appPort = Number(process.argv[4] ?? 43031);
const serverBin = path.join(root, 'target/debug/bruhs-server');
for (const [label, file] of [['bruhs-server', serverBin], ['web bundle', path.join(dist, 'index.html')]]) {
  if (!fs.existsSync(file)) {
    console.error(`${label} not found at ${file}; see the usage at the top of this script.`);
    process.exit(2);
  }
}
const cdpPort = Number(process.env.BRUHS_VERIFY_CDP_PORT ?? 9431);
const chromeBin = process.env.BRUHS_VERIFY_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const runDir = path.join(fs.realpathSync(tmpdir()), 'bruhs-crew-verify');
fs.rmSync(runDir, { recursive: true, force: true });
const home = path.join(runDir, 'home');
const bin = path.join(runDir, 'bin');
const shots = path.join(runDir, 'shots');
for (const dir of [home, bin, shots, path.join(runDir, 'chrome')]) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
console.log(`Run directory: ${runDir}`);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// A stand-in for the Claude Code CLI: signed in, starts a turn, streams one
// text delta, then waits until the server stops it. The chat bruh stays
// Working, so its face sweeps until Stop.
const standIn = path.join(bin, 'claude');
fs.writeFileSync(standIn, `#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true,"authMethod":"claude.ai","email":"verify@example.invalid"}'; exit 0 ;;
  --version|-v) echo "2.1.0 (Claude Code)"; exit 0 ;;
esac
printf '%s\\n' '{"type":"system","subtype":"init","session_id":"verify-crew-1","model":"claude-verify","tools":[],"mcp_servers":[]}'
printf '%s\\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Reading the notes folder"}}}'
while :; do sleep 1; done
`, { mode: 0o755 });

const portBusy = async (port) => {
  try {
    await fetch(`http://127.0.0.1:${port}/`, { signal: AbortSignal.timeout(500) });
    return true;
  } catch (error) {
    return error?.cause?.code !== 'ECONNREFUSED';
  }
};
for (const port of [appPort, cdpPort]) {
  if (await portBusy(port)) {
    console.error(`Port ${port} is in use; pick another port or set BRUHS_VERIFY_CDP_PORT.`);
    process.exit(2);
  }
}

const base = `http://127.0.0.1:${appPort}`;
const children = new Set();
async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once('exit', resolve));
  child.kill('SIGTERM');
  if (await Promise.race([exited.then(() => true), sleep(5000).then(() => false)]) === false) child.kill('SIGKILL');
  children.delete(child);
}
async function cleanup() {
  for (const child of [...children].reverse()) await stopChild(child);
  spawnSync('/usr/bin/pkill', ['-f', standIn]);
}
process.on('SIGINT', () => cleanup().then(() => process.exit(130)));

const results = [];
const record = (name, pass, detail = '') => {
  results.push({ name, pass: !!pass, detail: String(detail).slice(0, 400) });
  console.log(`${pass ? 'ok  ' : 'FAIL'} ${name}${detail ? ` (${String(detail).slice(0, 300)})` : ''}`);
  return !!pass;
};

let exitCode = 1;
try {
  const log = fs.openSync(path.join(runDir, 'server.log'), 'a');
  const server = spawn(serverBin, [], {
    env: {
      PATH: `${bin}:/usr/bin:/bin:/usr/sbin:/sbin`,
      HOME: home,
      TMPDIR: home,
      BRUHS_SERVER_PORT: String(appPort),
      BRUHS_WEB_DIST: dist,
      BRUHS_REACTIVE_DB: path.join(runDir, 'reactive.sqlite3'),
      BRUHS_CAPTURE_URL: 'http://127.0.0.1:9',
      BRUHS_CAPTURE_TOKEN_FILE: path.join(runDir, 'no-capture-token'),
      RUST_LOG: 'warn',
    },
    stdio: ['ignore', log, log],
  });
  children.add(server);
  for (let attempt = 0; ; attempt++) {
    try {
      if ((await fetch(`${base}/api/health`)).ok) break;
    } catch {}
    if (attempt > 150 || server.exitCode !== null) throw new Error('bruhs-server did not start; see server.log');
    await sleep(200);
  }

  const api = async (method, route, body) => {
    const response = await fetch(`${base}${route}`, {
      method,
      headers: { origin: base, ...(body ? { 'content-type': 'application/json' } : {}) },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!response.ok) throw new Error(`${method} ${route}: ${response.status} ${await response.text()}`);
    return response.json();
  };
  const command = (body) => api('POST', '/api/reactive/commands', body);
  const snapshot = () => api('GET', '/api/reactive');
  const until = async (label, test, timeout = 10000) => {
    const deadline = Date.now() + timeout;
    let last;
    for (;;) {
      last = await test();
      if (last) return last;
      if (Date.now() > deadline) throw new Error(`Timed out: ${label}`);
      await sleep(150);
    }
  };

  // ── Seed: three bruhs, a pending run for the Ask bruh, a chat ──────────────
  const bruh = (id, name, instructions, level, look) => ({
    action: 'create_bruh',
    agent: { id, name, instructions, model: 'claude-sonnet-4-6', project_path: '', enabled: true },
    look,
    access: { level, send_as_me: false, can_deliver: false },
  });
  const ids = { reader: 'crew-reader', asker: 'crew-asker', doer: 'crew-doer' };
  await command(bruh(ids.reader, 'Reader', 'Read the notes folder and summarise it.', 'read_only', { look: 'left', face: 'blueberry', mouth: 'bruh' }));
  await command(bruh(ids.asker, 'Asker', 'Tidy the inbox, asking before each change.', 'ask', { look: 'front', face: 'bruh', mouth: 'dragonfruit' }));
  await command(bruh(ids.doer, 'Doer', 'Keep the build green.', 'full', { look: 'right', face: 'mangosteen', mouth: 'longan' }));
  // A run that waits for review is never claimed by the worker, so face and
  // access edits act on a run whose state nothing else changes (ruling F17).
  await command({ action: 'apply', change: { type: 'save_source', source: { id: 'crew-inbox', name: 'Inbox', enabled: true, kind: { type: 'custom' } } } });
  await command({
    action: 'apply',
    change: {
      type: 'save_subscription',
      subscription: { id: 'crew-asker-inbox', source_id: 'crew-inbox', agent_id: ids.asker, instructions: 'Tidy what arrived.', enabled: true, auto_run: false, debounce_ms: 0, throttle_ms: 0 },
    },
  });
  await command({ action: 'publish', source_id: 'crew-inbox', key: 'mail-1', payload: { subject: 'Invoice' } });
  const pending = await until('the Ask bruh has a run waiting for review', async () =>
    (await snapshot()).runs.find((run) => run.agent_id === ids.asker && run.status === 'awaiting_review'));
  await command({ action: 'chat', agent_id: ids.reader, message: 'What changed in the notes today?' });
  await until('the chat bruh is working', async () =>
    (await snapshot()).bruhs.find((state) => state.agent_id === ids.reader)?.status === 'working');
  const runStatus = async (id) => (await snapshot()).runs.find((run) => run.id === id)?.status;

  // ── Chrome over CDP ─────────────────────────────────────────────────────────
  const chrome = spawn(chromeBin, ['--headless=new', `--remote-debugging-port=${cdpPort}`, `--user-data-dir=${path.join(runDir, 'chrome')}`, '--no-first-run', '--no-default-browser-check', '--disable-extensions', 'about:blank'], { stdio: 'ignore' });
  children.add(chrome);
  let targets = [];
  for (let attempt = 0; attempt < 100 && !targets.some((t) => t.type === 'page'); attempt++) {
    try {
      targets = await (await fetch(`http://127.0.0.1:${cdpPort}/json/list`)).json();
    } catch {}
    if (!targets.some((t) => t.type === 'page')) await sleep(200);
  }
  const ws = new WebSocket(targets.find((t) => t.type === 'page').webSocketDebuggerUrl);
  await new Promise((resolve) => ws.addEventListener('open', resolve, { once: true }));
  let nextId = 1;
  const waiting = new Map();
  const listeners = new Set();
  ws.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    if (message.id && waiting.has(message.id)) {
      const { resolve, reject } = waiting.get(message.id);
      waiting.delete(message.id);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result);
    } else if (message.method) {
      for (const listener of listeners) listener(message);
    }
  });
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = nextId++;
      waiting.set(id, { resolve, reject });
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails).slice(0, 500));
    return result.result.value;
  };
  const waitFor = async (label, expression, timeout = 10000) => {
    const deadline = Date.now() + timeout;
    for (;;) {
      const value = await evaluate(expression).catch(() => null);
      if (value) return value;
      if (Date.now() > deadline) throw new Error(`Timed out: ${label}`);
      await sleep(100);
    }
  };
  const js = JSON.stringify;
  // Click the centre of the first element matching `selector` (optionally
  // the one whose text is `text`), after checking nothing covers it.
  const click = async (selector, text) => {
    const point = await evaluate(`(() => {
      const all = [...document.querySelectorAll(${js(selector)})];
      const element = ${text === undefined ? 'all[0]' : `all.find((node) => node.textContent.trim() === ${js(text)})`};
      if (!element) return null;
      element.scrollIntoView({ block: 'center' });
      const box = element.getBoundingClientRect();
      const x = box.left + box.width / 2, y = box.top + box.height / 2;
      const hit = document.elementFromPoint(x, y);
      return { x, y, covered: !(hit && (hit === element || element.contains(hit))) };
    })()`);
    if (!point) throw new Error(`Nothing matches ${selector}${text === undefined ? '' : ` "${text}"`}`);
    if (point.covered) throw new Error(`${selector}${text === undefined ? '' : ` "${text}"`} is covered`);
    for (const type of ['mousePressed', 'mouseReleased']) {
      await send('Input.dispatchMouseEvent', { type, x: point.x, y: point.y, button: 'left', clickCount: 1 });
    }
  };
  const META = 4;
  const SHIFT = 8;
  const press = async (key, code, keyCode, modifiers = 0) => {
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode: keyCode, modifiers, ...(modifiers === 0 && (key.length === 1 || key === 'Enter') ? { text: key === 'Enter' ? '\r' : key } : {}) });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: keyCode, modifiers });
  };
  const viewport = (width, height, mobile = false) =>
    send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile });
  const shoot = async (name) => {
    await sleep(400);
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    const file = path.join(shots, `${name}.png`);
    fs.writeFileSync(file, Buffer.from(data, 'base64'));
    return file;
  };
  const rowSelector = (id) => `[data-crew-row=${js(id)}]`;
  const rowsText = () => evaluate(`[...document.querySelectorAll('[data-crew-row]')].map((row) => row.textContent.trim())`);

  // Any Rust panic in the page is a failure, whatever check was running.
  const panics = [];
  listeners.add((message) => {
    if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
      const text = message.params.args.map((arg) => arg.value ?? arg.description ?? '').join(' ');
      if (text.includes('panicked')) panics.push(text.slice(0, 300));
    }
  });
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Emulation.setFocusEmulationEnabled', { enabled: true });
  // Count animation frames the page asks for, from the first script on.
  await send('Page.addScriptToEvaluateOnNewDocument', {
    source: `window.__frames = 0; const request = window.requestAnimationFrame.bind(window);
      window.requestAnimationFrame = (callback) => { window.__frames += 1; return request(callback); };`,
  });
  await viewport(1280, 800);
  // A full load each time: a fragment-only navigation would keep the page.
  const openApp = async (hash = '') => {
    await send('Page.navigate', { url: 'about:blank' });
    await waitFor('a blank page', `location.href === 'about:blank'`);
    await send('Page.navigate', { url: `${base}/${hash}` });
    await waitFor('the app has loaded', `document.readyState === 'complete' && !!document.querySelector('[data-crew-rail]')`, 20000);
  };

  // 1. The rail lists the three bruhs above the thread list.
  await openApp();
  await waitFor('three rail rows', `document.querySelectorAll('[data-crew-row]').length === 3`);
  const order = await evaluate(`(() => {
    const rail = document.querySelector('[data-crew-rail]');
    const threads = document.querySelector('ul[aria-label="Threads"]');
    return {
      rows: [...rail.querySelectorAll('[data-crew-row]')].map((row) => row.getAttribute('data-crew-row')),
      above: !!threads && (rail.compareDocumentPosition(threads) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0
        && rail.getBoundingClientRect().bottom <= threads.getBoundingClientRect().top + 1,
    };
  })()`);
  record('rail lists three bruhs above the thread list',
    order.rows.length === 3 && Object.values(ids).every((id) => order.rows.includes(id)) && order.above,
    `rows ${order.rows.join(', ')}; above ${order.above}`);

  // 2. ⌘⇧B focuses the first row.
  await click('ul[aria-label="Threads"]').catch(() => evaluate('document.body.focus()'));
  await evaluate('document.activeElement && document.activeElement.blur && document.activeElement.blur()');
  await press('B', 'KeyB', 66, META | SHIFT);
  const focused = await waitFor('focus in the rail', `(() => {
    const first = document.querySelector('[data-crew-row]');
    return first && document.activeElement === first ? first.getAttribute('data-crew-row') : null;
  })()`, 3000).catch(() => null);
  record('⌘⇧B focuses the first row', !!focused, `focused ${focused ?? await evaluate(`document.activeElement?.outerHTML?.slice(0, 120)`)}`);

  // 3. A row opens #/bruhs/<id>, and the page does not stay on Loading.
  await click(rowSelector(ids.reader));
  const opened = await waitFor('the bruh page', `location.hash === ${js(`#/bruhs/${ids.reader}`)} && document.querySelector('#bruh-title')?.textContent.trim() === 'Reader'`, 5000).catch(() => false);
  record('a row opens #/bruhs/<id>', !!opened, await evaluate('location.hash'));
  // A link to another bruh, followed in the same page (back, forward or a
  // pasted link), switches the page.
  await evaluate(`location.hash = ${js(`#/bruhs/${ids.doer}`)}`);
  const switched = await waitFor('the linked bruh', `document.querySelector('#bruh-title')?.textContent.trim() === 'Doer'`, 5000).catch(() => false);
  record('a link to another bruh switches the page', !!switched, `title ${await evaluate(`document.querySelector('#bruh-title')?.textContent.trim()`)}`);
  await openApp(`#/bruhs/${ids.asker}`);
  await sleep(3000);
  const loading = await evaluate(`({
    loading: document.body.innerText.includes('Loading this bruh'),
    title: document.querySelector('#bruh-title')?.textContent.trim() ?? null,
  })`);
  record('a bruh page opened by its link loads with no stuck Loading', !loading.loading && loading.title === 'Asker', JSON.stringify(loading));

  // Screenshots: the rail with the chat bruh's Chat tab, and Settings.
  const screenshots = [];
  await openApp(`#/bruhs/${ids.reader}`);
  await waitFor('the chat transcript', `document.body.innerText.includes('What changed in the notes today?')`);
  for (const [width, height] of [[1280, 800], [640, 480]]) {
    await viewport(width, height);
    screenshots.push(await shoot(`web-chat-${width}x${height}`));
  }
  await viewport(1280, 800);
  await openApp(`#/bruhs/${ids.asker}`);
  await waitFor('the Asker page', `document.querySelector('#bruh-title')?.textContent.trim() === 'Asker'`);
  await click('[role="tab"]', 'Settings');
  await waitFor('the Settings form', `!!document.querySelector('form[aria-label="Bruh settings"]')`);
  for (const [width, height] of [[1280, 800], [640, 480]]) {
    await viewport(width, height);
    screenshots.push(await shoot(`web-settings-${width}x${height}`));
  }
  await viewport(1280, 800);
  record('screenshots saved', screenshots.every((file) => fs.statSync(file).size > 0), screenshots.map((file) => path.basename(file)).join(', '));

  // 4. Settings: Pandan and Save leave the pending run as it was.
  const saveButton = `form[aria-label="Bruh settings"] button`;
  await click('[role="radiogroup"][aria-label="Face colour"] [aria-label="Pandan"]');
  await click(saveButton, 'Save');
  const repainted = await until('the face edit is saved', async () =>
    (await snapshot()).bruhs.find((state) => state.agent_id === ids.asker)?.look.face === 'pandan').catch(() => false);
  await sleep(500);
  const afterFace = await runStatus(pending.id);
  record('picking Pandan and saving leaves the run pending', repainted && afterFace === 'awaiting_review', `face saved ${!!repainted}; run ${afterFace}`);

  // 5. Access to Full cancels it.
  await waitFor('Save settles', `[...document.querySelectorAll(${js(saveButton)})].find((b) => b.textContent.trim() === 'Save')?.disabled === true`);
  await click('[role="radiogroup"][aria-label="Access"] [role="radio"]', 'Full');
  await click(saveButton, 'Save');
  const afterAccess = await until('the access change cancels the run', async () => {
    const status = await runStatus(pending.id);
    return status === 'cancelled' ? status : null;
  }).catch(async () => runStatus(pending.id));
  const level = (await snapshot()).bruhs.find((state) => state.agent_id === ids.asker)?.access.level;
  record('changing access to Full cancels the run', afterAccess === 'cancelled' && level === 'full', `run ${afterAccess}; level ${level}`);

  // 6. ⌘. sends StopAll and every row says Paused; Resume all clears it.
  await press('.', 'Period', 190, META);
  const paused = await waitFor('every row paused', `(() => {
    const rows = [...document.querySelectorAll('[data-crew-row]')];
    return rows.length === 3 && rows.every((row) => row.textContent.includes('Paused'));
  })()`, 8000).catch(() => false);
  const server6 = await snapshot();
  record('⌘. pauses every bruh', paused && server6.bruhs.every((state) => state.status === 'paused'),
    `rows ${(await rowsText()).join(' | ')}; reader run ${server6.runs.filter((run) => run.agent_id === ids.reader).map((run) => run.status).join(',')}`);
  await click('[data-crew-resume-all]');
  const resumed = await waitFor('no row paused', `(() => {
    const rows = [...document.querySelectorAll('[data-crew-row]')];
    return rows.length === 3 && rows.every((row) => !row.textContent.includes('Paused'));
  })()`, 8000).catch(() => false);
  const server7 = await snapshot();
  record('Resume all clears it', resumed && server7.bruhs.every((state) => state.status !== 'paused') && !(await evaluate(`!!document.querySelector('[data-crew-resume-all]')`)),
    `rows ${(await rowsText()).join(' | ')}`);

  // 7. Reduced motion stops the sweep: no animation frames over a second.
  await command({ action: 'chat', agent_id: ids.reader, message: 'And yesterday?' });
  await waitFor('a working row', `[...document.querySelectorAll('[data-crew-row]')].some((row) => row.textContent.includes('Working'))`);
  const framesOverASecond = async () => {
    await evaluate('window.__frames = 0');
    await sleep(1000);
    return evaluate('window.__frames');
  };
  const moving = await framesOverASecond();
  await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
  await sleep(300);
  const still = await framesOverASecond();
  await openApp(`#/bruhs/${ids.reader}`);
  await waitFor('a working row', `[...document.querySelectorAll('[data-crew-row]')].some((row) => row.textContent.includes('Working'))`);
  await sleep(300);
  const stillAfterLoad = await framesOverASecond();
  record('reduced motion leaves no animation frames drawing', moving > 0 && still === 0 && stillAfterLoad === 0,
    `frames per second: moving ${moving}, reduced ${still}, reduced after reload ${stillAfterLoad}`);
  await send('Emulation.setEmulatedMedia', { features: [] });

  // 8. 390×844 touch: rail rows and swatches measure 44px.
  await viewport(390, 844, true);
  await send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
  await openApp(`#/bruhs/${ids.doer}`);
  await waitFor('the Doer page', `document.querySelector('#bruh-title')?.textContent.trim() === 'Doer'`);
  // Below sm the sidebar is a drawer over the page; close it with its scrim.
  // The drawer covers the scrim's middle, so tap its right edge.
  if (await evaluate(`!!document.querySelector('div.fixed.inset-0.z-30')`)) {
    for (const type of ['mousePressed', 'mouseReleased']) {
      await send('Input.dispatchMouseEvent', { type, x: 385, y: 600, button: 'left', clickCount: 1 });
    }
    await waitFor('the drawer closes', `!document.querySelector('div.fixed.inset-0.z-30')`, 3000);
  }
  await click('[role="tab"]', 'Settings');
  await waitFor('the swatches', `document.querySelectorAll('[role="radiogroup"][aria-label="Face colour"] [role="radio"]').length === 7`);
  const swatches = await evaluate(`[...document.querySelectorAll('[role="radiogroup"][aria-label$="colour"] [role="radio"]')].map((node) => { const box = node.getBoundingClientRect(); return [Math.round(box.width), Math.round(box.height)]; })`);
  screenshots.push(await shoot('web-settings-390x844-touch'));
  await press('B', 'KeyB', 66, META | SHIFT);
  await waitFor('the rail rows on a phone', `[...document.querySelectorAll('[data-crew-row]')].some((row) => row.getBoundingClientRect().height > 0)`, 3000);
  await sleep(300);
  const rows = await evaluate(`[...document.querySelectorAll('[data-crew-row]')].map((node) => Math.round(node.getBoundingClientRect().height))`);
  const coarse = await evaluate(`matchMedia('(pointer: coarse)').matches`);
  screenshots.push(await shoot('web-rail-390x844-touch'));
  record('390×844 touch: rail rows and swatches measure 44px',
    coarse && rows.length === 3 && rows.every((h) => h >= 44) && swatches.length === 12 && swatches.every(([w, h]) => w >= 44 && h >= 44),
    `coarse pointer ${coarse}; rows ${rows.join(',')}; swatches ${[...new Set(swatches.map((s) => s.join('x')))].join(',')}`);
  await send('Emulation.setTouchEmulationEnabled', { enabled: false });
  await viewport(1280, 800);

  // The rail is made again when the sidebar comes back, as after Settings.
  await openApp(`#/bruhs/${ids.reader}`);
  await evaluate(`location.hash = '#/settings'`);
  await waitFor('the sidebar hides on Settings', `!document.querySelector('[data-crew-rail]')`, 5000);
  // Let Settings finish mounting; it can write its own route once more.
  await sleep(1000);
  await evaluate(`location.hash = ${js(`#/bruhs/${ids.asker}`)}`);
  const back = await waitFor('the rail again', `document.querySelectorAll('[data-crew-row]').length === 3 && document.querySelector('#bruh-title')?.textContent.trim() === 'Asker'`, 5000).catch(() => false);
  record('the rail and a bruh page come back after Settings', !!back, JSON.stringify(await evaluate(`({
    rows: document.querySelectorAll('[data-crew-row]').length,
    rail: !!document.querySelector('[data-crew-rail]'),
    title: document.querySelector('#bruh-title')?.textContent.trim() ?? null,
    hash: location.hash,
  })`)));

  // 9. Create dialog: Enter in Name creates, and Cancel after Create stays put.
  await openApp(`#/bruhs/${ids.reader}`);
  const nameField = '[role="dialog"] input[aria-label="Name"]';
  const typeName = async (name) => {
    await click('[data-crew-new]');
    await waitFor('the create dialog', `!!document.querySelector(${js(nameField)})`);
    await evaluate(`document.querySelector(${js(nameField)}).focus()`);
    await send('Input.insertText', { text: name });
  };
  await typeName('Enter bruh');
  await press('Enter', 'Enter', 13);
  const created = await until('Enter creates the bruh', async () =>
    (await snapshot()).agents.find((agent) => agent.name === 'Enter bruh'), 4000).catch(() => null);
  const openedNew = created && await waitFor('the new bruh opens', `location.hash === ${js(`#/bruhs/${created.id}`)}`, 4000).catch(() => false);
  record('Enter in the create dialog Name creates', !!created && !!openedNew, created ? `created ${created.id}; hash ${await evaluate('location.hash')}` : 'no bruh created');
  if (!created) await evaluate(`[...document.querySelectorAll('[role="dialog"] button')].find((b) => b.textContent.trim() === 'Cancel')?.click()`);

  const before = await evaluate('location.hash');
  await send('Fetch.enable', { patterns: [{ urlPattern: '*/api/reactive/commands*', requestStage: 'Request' }] });
  const held = new Promise((resolve) => {
    const listener = (message) => {
      if (message.method === 'Fetch.requestPaused') {
        listeners.delete(listener);
        resolve(message.params.requestId);
      }
    };
    listeners.add(listener);
  });
  await typeName('Cancel bruh');
  await click('[role="dialog"] button', 'Create');
  const requestId = await Promise.race([held, sleep(4000).then(() => null)]);
  await click('[role="dialog"] button', 'Cancel');
  const closed = await waitFor('the dialog closes', `!document.querySelector(${js(nameField)})`, 3000).catch(() => false);
  if (requestId) await send('Fetch.continueRequest', { requestId });
  await send('Fetch.disable');
  const cancelled = await until('the held create lands', async () =>
    (await snapshot()).agents.find((agent) => agent.name === 'Cancel bruh'), 4000).catch(() => null);
  await sleep(1500);
  const after = await evaluate('location.hash');
  record('Cancel after Create does not navigate later', !!requestId && !!closed && !!cancelled && after === before,
    `held ${!!requestId}; closed ${!!closed}; created ${cancelled?.id ?? 'no'}; hash ${before} -> ${after}`);

  record('no panics in the page', panics.length === 0, panics.join(' | '));

  exitCode = results.every((result) => result.pass) ? 0 : 1;
  fs.writeFileSync(path.join(runDir, 'results.json'), JSON.stringify({ when: new Date().toISOString(), base, results, screenshots }, null, 2));
  console.log(`Screenshots: ${shots}`);
  console.log(exitCode === 0 ? 'All crew checks passed.' : `${results.filter((r) => !r.pass).length} crew check(s) failed.`);
} catch (error) {
  console.error(`FAIL ${error.stack ?? error}`);
  fs.writeFileSync(path.join(runDir, 'results.json'), JSON.stringify({ when: new Date().toISOString(), results, error: String(error) }, null, 2));
} finally {
  await cleanup();
}
process.exit(exitCode);
