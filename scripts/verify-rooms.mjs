// Acceptance run for rooms on the web (docs/crew.md "Rooms"; PARITY-17 in
// docs/web-desktop-parity-goal-2026-11-15.md): the rail's rooms, New room
// by keyboard, who a post reaches, hand-offs and their limit, paused
// members, approval and question cards in a room, Stop all, Room settings
// (rename, Delete room and its confirm), phone-size touch targets, Load
// earlier and reduced motion. Any panic in
// the page fails the run. Every check prints ok or FAIL; the exit status is
// 0 only when all pass.
//
//   cargo build -p bruhs-server
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43041 trunk build --dist "$TMPDIR/bruhs-rooms-dist")
//   node scripts/verify-rooms.mjs "$PWD" "$TMPDIR/bruhs-rooms-dist" 43041
//
// The bundle must be built for the port the check uses. bruhs-server runs
// with a disposable HOME and BRUHS_REACTIVE_DB, a capture URL nothing listens
// on and, first on its PATH, a stand-in `claude`, so no provider is called
// and no real data is touched. The stand-in reads the run's first stdin line,
// finds "Your name is <name>." and answers as that bruh (see `replies`); a
// file `hold-<name>` beside it keeps that bruh working until it is removed.
// The one line no run can write, a card whose outbox item is gone (Decided),
// is put straight into the room's table with sqlite3.
// Headless Chrome (BRUHS_VERIFY_CHROME, default the macOS app) runs on
// BRUHS_VERIFY_CDP_PORT (default 9441) with its own profile. Screenshots,
// logs and results go to $TMPDIR/bruhs-rooms-verify/, cleared on each run.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const root = path.resolve(process.argv[2] ?? '');
const dist = path.resolve(process.argv[3] ?? '');
const appPort = Number(process.argv[4] ?? 43041);
const serverBin = path.join(root, 'target/debug/bruhs-server');
for (const [label, file] of [['bruhs-server', serverBin], ['web bundle', path.join(dist, 'index.html')]]) {
  if (!fs.existsSync(file)) {
    console.error(`${label} not found at ${file}; see the usage at the top of this script.`);
    process.exit(2);
  }
}
const cdpPort = Number(process.env.BRUHS_VERIFY_CDP_PORT ?? 9441);
const chromeBin = process.env.BRUHS_VERIFY_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const sqlite = '/usr/bin/sqlite3';
const runDir = path.join(fs.realpathSync(tmpdir()), 'bruhs-rooms-verify');
fs.rmSync(runDir, { recursive: true, force: true });
const home = path.join(runDir, 'home');
const bin = path.join(runDir, 'bin');
const shots = path.join(runDir, 'shots');
for (const dir of [home, bin, shots, path.join(runDir, 'chrome')]) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
const database = path.join(runDir, 'reactive.sqlite3');
console.log(`Run directory: ${runDir}`);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// A stand-in for the Claude Code CLI. Each bruh answers by name: Ada hands
// to Bo, Bo checks, Pip and Pop hand to each other, Cy asks to write a file
// and waits for the owner's decision, Quinn asks the owner a question, Fay
// fails. A resumed run (the owner answered) says thanks. calls.log keeps
// every prompt it was given.
const replies = {
  Ada: '@Bo can you check the notes?',
  Bo: 'Checked. All good.',
  Pip: '@Pop your turn',
  Pop: '@Pip your turn',
  Quinn: '```bruhs-report\n{"outcome":"ask","title":"Which folder?","message":"Which folder should I tidy?"}\n```',
};
const standIn = path.join(bin, 'claude');
fs.writeFileSync(standIn, `#!${process.execPath}
const fs = require('fs');
const path = require('path');
const readline = require('readline');
const args = process.argv.slice(2);
if (args[0] === 'auth') {
  console.log(JSON.stringify({ loggedIn: true, authMethod: 'claude.ai', email: 'verify@example.invalid' }));
  process.exit(0);
}
if (args[0] === '--version' || args[0] === '-v') {
  console.log('2.1.0 (Claude Code)');
  process.exit(0);
}
const replies = ${JSON.stringify(replies)};
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const out = (event) => process.stdout.write(JSON.stringify(event) + '\\n');
const lines = readline.createInterface({ input: process.stdin })[Symbol.asyncIterator]();
const next = async () => (await lines.next()).value ?? '';
const text = (line) => {
  try {
    const content = JSON.parse(line).message.content;
    return typeof content === 'string' ? content : content.map((part) => part.text ?? '').join('');
  } catch {
    return '';
  }
};
(async () => {
  const prompt = text(await next());
  const name = (prompt.match(/Your name is ([^.\\n]+)\\./) ?? [])[1] ?? 'Unknown';
  fs.appendFileSync(path.join(__dirname, 'calls.log'), JSON.stringify({ name, at: Date.now(), prompt }) + '\\n');
  const session = 'verify-rooms-' + name.toLowerCase();
  out({ type: 'system', subtype: 'init', session_id: session, model: 'claude-verify', tools: [], mcp_servers: [] });
  while (fs.existsSync(path.join(__dirname, 'hold-' + name))) await sleep(150);
  let answer = replies[name] ?? 'Done.';
  let failed = false;
  if (/^The owner (approved|replied)/m.test(prompt)) {
    answer = 'Thanks, on it.';
  } else if (name === 'Cy') {
    out({ type: 'control_request', request_id: 'cy-' + Date.now(), request: { subtype: 'can_use_tool', tool_name: 'Write', input: { file_path: '/verify/notes-' + Date.now() + '.md', content: 'Notes' }, tool_use_id: 'toolu_cy' } });
    let decision = '';
    while (!decision.includes('control_response')) decision = await next();
    answer = decision.includes('"allow"') ? 'Wrote it.' : 'Skipped it.';
  } else if (name === 'Fay') {
    answer = 'Ran out of road.';
    failed = true;
  }
  out({ type: 'result', subtype: failed ? 'error_during_execution' : 'success', is_error: failed, result: answer, session_id: session, duration_ms: 1 });
  process.exit(0);
})();
`, { mode: 0o755 });
const hold = (name) => fs.writeFileSync(path.join(bin, `hold-${name}`), '');
const release = (name) => fs.rmSync(path.join(bin, `hold-${name}`), { force: true });

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
  for (const file of fs.readdirSync(bin)) if (file.startsWith('hold-')) fs.rmSync(path.join(bin, file), { force: true });
  for (const child of [...children].reverse()) await stopChild(child);
  spawnSync('/usr/bin/pkill', ['-f', standIn]);
}
process.on('SIGINT', () => cleanup().then(() => process.exit(130)));

const results = [];
const record = (name, pass, detail = '') => {
  results.push({ name, pass: !!pass, detail: String(detail).slice(0, 600) });
  console.log(`${pass ? 'ok  ' : 'FAIL'} ${name}${detail ? ` (${String(detail).slice(0, 400)})` : ''}`);
  return !!pass;
};
// A check that throws fails on its own; the run goes on to the next one.
const check = async (name, body) => {
  try {
    await body();
  } catch (error) {
    record(name, false, error?.message ?? String(error));
  }
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
      BRUHS_REACTIVE_DB: database,
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
  // A room's messages, oldest first.
  const messages = async (roomId) =>
    (await api('GET', `/api/reactive/rooms/${encodeURIComponent(roomId)}/messages`)).sort((a, b) => a.seq - b.seq);
  const said = async (roomId, test) => (await messages(roomId)).filter(test);
  const roomRuns = (snap, roomId) => snap.runs.filter((run) => run.trigger?.type === 'room' && run.trigger.room_id === roomId);
  const live = (run) => run.status === 'queued' || run.status === 'running';
  // Wait until nothing is queued or running in the room.
  const settled = (roomId, timeout = 15000) =>
    until(`room ${roomId} settles`, async () => !roomRuns(await snapshot(), roomId).some(live), timeout);

  // ── Seed: eight bruhs and two rooms, made out of name order ────────────────
  const looks = [['left', 'blueberry', 'bruh'], ['front', 'bruh', 'dragonfruit'], ['right', 'mangosteen', 'longan'], ['front', 'pandan', 'bruh']];
  const ids = {};
  const names = ['Ada', 'Bo', 'Cy', 'Pip', 'Pop', 'Quinn', 'Fay'];
  for (const [index, name] of names.entries()) {
    const id = `room-${name.toLowerCase()}`;
    ids[name] = id;
    const [look, face, mouth] = looks[index % looks.length];
    await command({
      action: 'create_bruh',
      agent: { id, name, instructions: `${name} works in rooms.`, model: 'claude-sonnet-4-6', project_path: '', enabled: true },
      look: { look, face, mouth },
      access: { level: name === 'Cy' ? 'ask' : 'read_only', send_as_me: false, can_deliver: false },
    });
  }
  const rooms = { pingPong: 'room-ping-pong', cards: 'room-cards' };
  await command({ action: 'create_room', room: { id: rooms.pingPong, name: 'Ping pong', members: [ids.Pip, ids.Pop], lead: null } });
  await command({ action: 'create_room', room: { id: rooms.cards, name: 'Cards', members: [ids.Cy, ids.Quinn, ids.Fay], lead: null } });

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
      return { x, y, covered: !(hit && (hit === element || element.contains(hit))), disabled: !!element.disabled };
    })()`);
    if (!point) throw new Error(`Nothing matches ${selector}${text === undefined ? '' : ` "${text}"`}`);
    if (point.covered) throw new Error(`${selector}${text === undefined ? '' : ` "${text}"`} is covered`);
    if (point.disabled) throw new Error(`${selector}${text === undefined ? '' : ` "${text}"`} is disabled`);
    for (const type of ['mousePressed', 'mouseReleased']) {
      await send('Input.dispatchMouseEvent', { type, x: point.x, y: point.y, button: 'left', clickCount: 1 });
    }
  };
  const META = 4;
  const SHIFT = 8;
  const keys = {
    Tab: ['Tab', 9], Enter: ['Enter', 13], Escape: ['Escape', 27], ' ': ['Space', 32],
    a: ['KeyA', 65], B: ['KeyB', 66], '.': ['Period', 190],
  };
  const press = async (key, modifiers = 0) => {
    const [code, keyCode] = keys[key];
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode: keyCode, modifiers, ...(modifiers === 0 && (key.length === 1 || key === 'Enter') ? { text: key === 'Enter' ? '\r' : key } : {}) });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: keyCode, modifiers });
  };
  const type = (text) => send('Input.insertText', { text });
  const viewport = (width, height, mobile = false) =>
    send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile });
  const screenshots = [];
  const shoot = async (name) => {
    await sleep(400);
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    const file = path.join(shots, `${name}.png`);
    fs.writeFileSync(file, Buffer.from(data, 'base64'));
    screenshots.push(file);
    return file;
  };
  const active = () => evaluate(`(() => {
    const element = document.activeElement;
    return element ? { tag: element.tagName, member: element.getAttribute('data-room-member'), label: element.getAttribute('aria-label'), role: element.getAttribute('role'), type: element.getAttribute('type'), text: element.textContent.trim().slice(0, 40), keep: element.hasAttribute('data-room-delete-keep'), del: element.hasAttribute('data-room-delete') } : null;
  })()`);

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
  const openRoom = async (roomId, name) => {
    await openApp(`#/rooms/${roomId}`);
    await waitFor(`the ${name} room`, `document.querySelector('#room-title')?.textContent.trim() === ${js(name)}`);
  };
  const composer = '[data-room-composer] textarea';
  const sendButton = '[data-room-composer] button';
  const post = async (text) => {
    await evaluate(`document.querySelector(${js(composer)}).focus()`);
    await type(text);
    await click(sendButton, 'Send');
  };

  // 1. The rail lists the seeded rooms under the bruhs, by name.
  await check('the rail lists the seeded rooms under the bruhs, by name', async () => {
    await openApp();
    await waitFor('two room rows', `document.querySelectorAll('[data-crew-room]').length === 2`);
    const rail = await evaluate(`(() => {
      const bruhs = document.querySelector('ul[aria-label="Bruhs"]');
      const rooms = document.querySelector('ul[aria-label="Rooms"]');
      return {
        rooms: [...document.querySelectorAll('[data-crew-room]')].map((row) => row.textContent.trim()),
        bruhs: document.querySelectorAll('[data-crew-row]').length,
        under: !!bruhs && !!rooms && (bruhs.compareDocumentPosition(rooms) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0
          && bruhs.getBoundingClientRect().bottom <= rooms.getBoundingClientRect().top + 1,
      };
    })()`);
    record('the rail lists the seeded rooms under the bruhs, by name',
      rail.rooms.length === 2 && rail.rooms[0].startsWith('Cards') && rail.rooms[1].startsWith('Ping pong') && rail.under && rail.bruhs === names.length,
      `rooms ${rail.rooms.join(' | ')}; bruhs ${rail.bruhs}; under ${rail.under}`);
  });
  await shoot('web-rail-1280x800');

  // 2. New room by keyboard only. Escape in the Lead list closes the list,
  // not the dialog.
  let launch = null;
  await check('New room by keyboard creates Launch with Ada as lead and Bo, and opens it', async () => {
    await evaluate(`document.querySelector('[data-crew-new-room]').focus()`);
    await press('Enter');
    await waitFor('the New room dialog', `document.activeElement?.getAttribute('aria-label') === 'Name' && !!document.activeElement.closest('[data-room-dialog]')`, 3000);
    await type('Launch');
    const checked = [];
    await press('Tab');
    for (let step = 0; step < names.length; step++) {
      const now = await active();
      if (!now?.member) break;
      if (now.member === ids.Ada || now.member === ids.Bo) {
        await press(' ');
        checked.push(now.member);
      }
      await press('Tab');
    }
    const onLead = await active();
    if (onLead?.label !== 'Lead') throw new Error(`after the members focus is on ${JSON.stringify(onLead)}`);
    await press('Enter');
    await waitFor('the Lead list', `document.activeElement?.getAttribute('role') === 'listbox'`, 3000);
    await press('Escape');
    const escaped = await waitFor('the Lead list closes', `!document.querySelector('[data-room-dialog] [role="listbox"]') && document.activeElement?.getAttribute('aria-label') === 'Lead' ? 'closed' : null`, 3000).catch(() => null);
    const dialogStays = await evaluate(`!!document.querySelector('[data-room-dialog]')`);
    record('Escape in the Lead list closes only the list', !!escaped && dialogStays, `list closed ${!!escaped}; dialog open ${dialogStays}; focus ${JSON.stringify(await active())}`);
    await press('Enter');
    await waitFor('the Lead list again', `document.activeElement?.getAttribute('role') === 'listbox'`, 3000);
    await press('a');
    await press('Enter');
    await waitFor('the lead reads Ada', `document.querySelector('[data-room-dialog] button[aria-label="Lead"]')?.textContent.trim() === 'Ada'`, 3000);
    for (let step = 0; step < 6; step++) {
      await press('Tab');
      const now = await active();
      if (now?.type === 'submit') break;
    }
    const submit = await active();
    if (submit?.type !== 'submit') throw new Error(`Tab never reached Create room: ${JSON.stringify(submit)}`);
    await press('Enter');
    launch = await until('Launch is created', async () => (await snapshot()).rooms.find((state) => state.room.name === 'Launch')?.room, 5000);
    const opened = await waitFor('the Launch page', `location.hash === ${js(`#/rooms/${launch.id}`)} && document.querySelector('#room-title')?.textContent.trim() === 'Launch'`, 5000).catch(() => false);
    record('New room by keyboard creates Launch with Ada as lead and Bo, and opens it',
      checked.length === 2 && launch.lead === ids.Ada && launch.members.length === 2 && launch.members.includes(ids.Ada) && launch.members.includes(ids.Bo) && !!opened,
      `checked ${checked.join(',')}; room ${JSON.stringify(launch)}; hash ${await evaluate('location.hash')}`);
  });
  if (!launch) throw new Error('Launch was not created; the checks after this need it.');

  // 3. A post with no mention reaches the lead.
  await check('a post with no mention reaches the lead, whose reply shows with her face', async () => {
    await post('Plan the launch.');
    const ada = await waitFor("Ada's reply", `(() => {
      const line = [...document.querySelectorAll('[data-room-author=${js(ids.Ada)}]')].find((node) => node.textContent.includes('@Bo can you check the notes?'));
      if (!line) return null;
      const face = line.firstElementChild;
      return { face: face?.tagName === 'CANVAS' && face.width > 0, name: line.textContent.trim().startsWith('Ada') };
    })()`, 10000);
    const runs = roomRuns(await snapshot(), launch.id);
    record('a post with no mention reaches the lead, whose reply shows with her face',
      ada.name && ada.face && runs.some((run) => run.agent_id === ids.Ada && run.status === 'completed'),
      `face ${ada.face}; name ${ada.name}; runs ${runs.map((run) => `${run.agent_id}:${run.status}`).join(',')}`);
  });

  // 4. Ada's @Bo hands off; Bo's chat shows the run marked Room.
  await check("Ada's @Bo hands off to Bo, whose chat shows the run marked Room", async () => {
    await waitFor("Bo's reply", `[...document.querySelectorAll('[data-room-author=${js(ids.Bo)}]')].some((node) => node.textContent.includes('Checked. All good.'))`, 10000);
    const order = (await messages(launch.id)).map((message) => message.author.type === 'bruh' ? message.author.agent_id : message.author.type);
    const after = order.indexOf(ids.Bo) > order.indexOf(ids.Ada) && order.indexOf(ids.Ada) > order.indexOf('owner');
    const bo = roomRuns(await snapshot(), launch.id).find((run) => run.agent_id === ids.Bo);
    await settled(launch.id);
    await shoot('web-room-1280x800');
    await viewport(640, 480);
    await shoot('web-room-640x480');
    await viewport(1280, 800);
    await openApp(`#/bruhs/${ids.Bo}`);
    const chat = await waitFor("Bo's chat", `(() => {
      const labels = [...document.querySelectorAll('span')].filter((span) => span.textContent.trim() === 'Room');
      return document.body.innerText.includes('Checked. All good.') && labels.length > 0 ? labels.length : null;
    })()`, 8000).catch(() => null);
    record("Ada's @Bo hands off to Bo, whose chat shows the run marked Room", after && bo?.trigger?.hop === 1 && !!chat,
      `order ${order.join(' > ')}; Bo hop ${bo?.trigger?.hop}; Room labels ${chat}`);
  });

  // 5. @b lists Bo first; Enter inserts the chip; Send posts and clears.
  await check('typing @b lists Bo first, Enter inserts the chip and Send posts and clears', async () => {
    await openRoom(launch.id, 'Launch');
    await evaluate(`document.querySelector(${js(composer)}).focus()`);
    await type('@');
    await type('b');
    const first = await waitFor('the @ list', `document.querySelector('[data-mention-popover] [data-mention-row="0"]')?.textContent.trim() ?? null`, 3000);
    await press('Enter');
    const inserted = await waitFor('the chip', `(() => {
      const value = document.querySelector(${js(composer)}).value;
      return value.startsWith('@Bo') && !document.querySelector('[data-mention-popover]') ? value : null;
    })()`, 3000).catch(() => null);
    await type('check the list');
    const before = (await messages(launch.id)).length;
    await click(sendButton, 'Send');
    const cleared = await waitFor('the composer clears', `document.querySelector(${js(composer)}).value === ''`, 3000).catch(() => false);
    const posted = await until('the post lands', async () =>
      (await messages(launch.id)).slice(before).find((message) => message.author.type === 'owner'), 5000).catch(() => null);
    const reached = posted && await until('Bo gets it', async () =>
      roomRuns(await snapshot(), launch.id).find((run) => run.trigger.message_id === posted.id), 5000).catch(() => null);
    record('typing @b lists Bo first, Enter inserts the chip and Send posts and clears',
      first.startsWith('Bo') && !!inserted && !!cleared && posted?.text.startsWith('@Bo') && reached?.agent_id === ids.Bo,
      `first row ${first}; inserted ${js(inserted)}; cleared ${!!cleared}; posted ${js(posted?.text)}; run for ${reached?.agent_id}`);
    await settled(launch.id);
  });

  // 7. A paused member gets a System line instead of a run.
  await check('a paused member gets "… is paused, so it did not get this message."', async () => {
    await command({ action: 'stop_bruh', agent_id: ids.Bo });
    await until('Bo is paused', async () => (await snapshot()).bruhs.find((state) => state.agent_id === ids.Bo)?.status === 'paused');
    await post('@Bo are you there?');
    const line = 'Bo is paused, so it did not get this message.';
    const shown = await waitFor('the paused line', `[...document.querySelectorAll('[data-room-line="system"]')].some((node) => node.textContent.trim() === ${js(line)})`, 5000).catch(() => false);
    const runs = roomRuns(await snapshot(), launch.id).filter((run) => run.agent_id === ids.Bo && live(run));
    record('a paused member gets "… is paused, so it did not get this message."', !!shown && runs.length === 0, `shown ${!!shown}; live Bo runs ${runs.length}`);
    await command({ action: 'resume_bruh', agent_id: ids.Bo });
  });

  // 6. Ping pong stops after six hand-offs, with one line saying so.
  await check('the ping-pong room stops after 6 hand-offs with one "Stopped handing off" line', async () => {
    await openRoom(rooms.pingPong, 'Ping pong');
    await post('@Pip serve.');
    const stop = 'Stopped handing off after 6 hand-offs in a row. Post here to keep going.';
    await until('the hand-off limit', async () => (await said(rooms.pingPong, (message) => message.text === stop)).length > 0, 20000);
    await settled(rooms.pingPong);
    await sleep(1000);
    const all = await messages(rooms.pingPong);
    const replies = all.filter((message) => message.author.type === 'bruh');
    const stops = all.filter((message) => message.text === stop);
    const hops = roomRuns(await snapshot(), rooms.pingPong).map((run) => run.trigger.hop).sort((a, b) => a - b);
    const shown = await waitFor('the line on the page', `[...document.querySelectorAll('[data-room-line="system"]')].filter((node) => node.textContent.trim() === ${js(stop)}).length`, 5000).catch(() => 0);
    record('the ping-pong room stops after 6 hand-offs with one "Stopped handing off" line',
      stops.length === 1 && shown === 1 && replies.length === 7 && hops.at(-1) === 6 && all.at(-1).text === stop,
      `replies ${replies.length}; stop lines ${stops.length} (page ${shown}); hops ${hops.join(',')}`);
  });

  // 8. Cy's approval is a card in the room and in Cy's chat; Approve in the
  // room resumes Cy, whose reply lands in the room.
  await check("Cy's approval shows in the room and in Cy's chat; Approve in the room resumes Cy", async () => {
    await openRoom(rooms.cards, 'Cards');
    await post('@Cy write the notes.');
    const card = `[data-room-author=${js(ids.Cy)}] article[aria-label="Approval"]`;
    const inRoom = await waitFor('the approval card in the room', `(() => {
      const card = document.querySelector(${js(card)});
      const buttons = card ? [...card.querySelectorAll('button')].map((button) => button.textContent.trim()) : [];
      return buttons.includes('Approve') && buttons.includes('Deny') ? card.textContent.slice(0, 120) : null;
    })()`, 10000);
    await openApp(`#/bruhs/${ids.Cy}`);
    const inChat = await waitFor("the card in Cy's chat", `[...document.querySelectorAll('article[aria-label="Approval"]')].some((card) => [...card.querySelectorAll('button')].some((button) => button.textContent.trim() === 'Approve'))`, 8000).catch(() => false);
    await openRoom(rooms.cards, 'Cards');
    await waitFor('the room card again', `!!document.querySelector(${js(card)})`);
    await click(`${card} button`, 'Approve');
    const reply = await waitFor("Cy's reply in the room", `[...document.querySelectorAll('[data-room-author=${js(ids.Cy)}] [data-room-line="bruh"]')].some((node) => node.textContent.trim() === 'Wrote it.')`, 10000).catch(() => false);
    const decided = await evaluate(`(() => { const card = document.querySelector(${js(card)}); return card ? ![...card.querySelectorAll('button')].some((button) => button.textContent.trim() === 'Approve') : false; })()`);
    record("Cy's approval shows in the room and in Cy's chat; Approve in the room resumes Cy",
      !!inRoom && !!inChat && !!reply && decided,
      `room card ${js(inRoom)}; chat card ${!!inChat}; reply ${!!reply}; card settled ${decided}`);
    await settled(rooms.cards);
  });

  // Seeded cards: a question card answered in the room, a card whose item is
  // gone (Decided) and a failed reply, each drawn as the shared rules say.
  await check('a question card, the decided card and a failed reply render in a room, and Reply works', async () => {
    await openRoom(rooms.cards, 'Cards');
    await post('@Quinn tidy the inbox.');
    const question = `[data-room-author=${js(ids.Quinn)}] article[aria-label="Question"]`;
    await waitFor('the question card', `!!document.querySelector(${js(`${question} textarea`)})`, 10000);
    await evaluate(`document.querySelector(${js(`${question} textarea`)}).focus()`);
    await type('Use notes.');
    await click(`${question} button`, 'Reply');
    const answered = await waitFor('the answered card', `document.querySelector(${js(question)})?.textContent.includes('You answered: Use notes.')`, 8000).catch(() => false);
    const thanks = await until("Quinn's resumed reply in the room", async () =>
      (await said(rooms.cards, (message) => message.author.agent_id === ids.Quinn && message.text === 'Thanks, on it.')).length === 1, 10000).catch(() => false);
    const quinnLines = (await said(rooms.cards, (message) => message.author.agent_id === ids.Quinn)).map((message) => (message.outbox_id ? 'card' : message.text));

    await post('@Fay try the build.');
    const failed = await waitFor('the failed reply', `(() => {
      const line = [...document.querySelectorAll('[data-room-author=${js(ids.Fay)}] [data-room-line="bruh"]')].find((node) => node.className.includes('lychee'));
      return line ? { text: line.textContent.trim(), buttons: line.closest('[data-room-author]').querySelectorAll('button').length } : null;
    })()`, 10000).catch(() => null);
    await settled(rooms.cards);

    // A question whose outbox item has gone reads as Decided.
    const seq = Math.max(0, ...(await messages(rooms.cards)).map((message) => message.seq)) + 1;
    const decidedMessage = { id: 'seed-decided', room_id: rooms.cards, seq, author: { type: 'bruh', agent_id: ids.Cy }, text: 'Send the weekly note', run_id: null, outbox_id: 'seed-gone-item', failed: false, created_at: Date.now() };
    const insert = spawnSync(sqlite, [database, '.timeout 5000', `INSERT INTO reactive_room_messages(id,room_id,seq,result_of,payload) VALUES('seed-decided','${rooms.cards}',${seq},NULL,'${JSON.stringify(decidedMessage).replaceAll("'", "''")}');`], { encoding: 'utf8' });
    if (insert.status !== 0) throw new Error(`seeding the decided card failed: ${insert.stderr}`);
    await openRoom(rooms.cards, 'Cards');
    const decided = await waitFor('the decided card', `(() => {
      const card = document.querySelector('[data-room-author=${js(ids.Cy)}] article[data-room-line="asked"]');
      return card ? { text: card.textContent.trim(), buttons: card.querySelectorAll('button').length } : null;
    })()`, 8000).catch(() => null);
    const approval = await evaluate(`document.querySelector('[data-room-author=${js(ids.Cy)}] article[aria-label="Approval"]:not([data-room-line])')?.textContent.trim() ?? null`);
    await shoot('web-room-cards-1280x800');
    record('a question card, the decided card and a failed reply render in a room, and Reply works',
      !!answered && !!thanks && quinnLines[0] === 'card' && quinnLines.length === 2
        && failed?.text === 'The provider reported a failed turn.' && failed.buttons === 0
        && decided?.text.startsWith('Wants to') && decided.text.includes('Send the weekly note') && decided.text.endsWith('Decided') && decided.buttons === 0
        && !!approval,
      `answered ${!!answered}; Quinn lines ${quinnLines.join(' | ')}; failed ${js(failed)}; decided ${js(decided)}; approval card ${js(approval?.slice(0, 80))}`);
  });

  // 9. Stop all mid hand-off: each live room run ends with one line.
  await check('Stop all mid hand-off ends each live room run with one "did not answer" line', async () => {
    hold('Bo');
    hold('Pop');
    await openRoom(launch.id, 'Launch');
    await post('Plan the next launch.');
    await command({ action: 'post_room', room_id: rooms.pingPong, text: '@Pip serve again.' });
    const caught = await until('Bo and Pop are working mid hand-off', async () => {
      const snap = await snapshot();
      const runs = [...roomRuns(snap, launch.id), ...roomRuns(snap, rooms.pingPong)].filter(live);
      return runs.some((run) => run.agent_id === ids.Bo && run.status === 'running')
        && runs.some((run) => run.agent_id === ids.Pop && run.status === 'running') ? runs : null;
    }, 15000);
    await waitFor('a working line', `!!document.querySelector('[data-room-line="working"]')`, 3000);
    await press('.', META);
    await until('every bruh is paused', async () => (await snapshot()).bruhs.every((state) => state.status === 'paused'), 8000);
    await sleep(1500);
    const lines = [];
    for (const run of caught) {
      const roomId = run.trigger.room_id;
      lines.push({ run, said: await said(roomId, (message) => message.run_id === run.id && message.text.includes('did not answer')) });
    }
    const each = lines.every(({ said }) => said.length === 1 && said[0].author.type === 'system');
    const shown = await waitFor('the stopped line on the page', `[...document.querySelectorAll('[data-room-line="system"]')].filter((node) => node.textContent.includes('did not answer')).length`, 5000).catch(() => 0);
    const working = await evaluate(`document.querySelectorAll('[data-room-line="working"]').length`);
    record('Stop all mid hand-off ends each live room run with one "did not answer" line',
      caught.length >= 2 && each && shown === lines.filter(({ run }) => run.trigger.room_id === launch.id).length && working === 0,
      lines.map(({ run, said }) => `${run.agent_id} ${run.status}: ${said.map((message) => message.text).join(' / ') || 'none'}`).join('; ') + `; page ${shown}; working rows ${working}`);
    release('Bo');
    release('Pop');
    await click('[data-crew-resume-all]');
    await until('every bruh resumes', async () => (await snapshot()).bruhs.every((state) => state.status !== 'paused'), 8000);
  });

  // 10. Room settings: a rename keeps queued runs; Delete room asks first,
  // its confirm moves focus to Keep and back, and Delete removes the row.
  await check('renaming in Room settings keeps queued runs', async () => {
    hold('Bo');
    await openRoom(launch.id, 'Launch');
    await post('@Bo first look.');
    await until("Bo's first run is running", async () => roomRuns(await snapshot(), launch.id).some((run) => run.agent_id === ids.Bo && run.status === 'running'), 8000);
    await post('@Bo second look.');
    const queued = await until('a second Bo run queues', async () => {
      const runs = roomRuns(await snapshot(), launch.id).filter((run) => run.agent_id === ids.Bo && live(run));
      return runs.length === 2 && runs.some((run) => run.status === 'queued') ? runs : null;
    }, 8000);
    // The rail keeps its own copy of the crew, so it catches up on the
    // service's next frame rather than with the page.
    const railStart = Date.now();
    const working = await waitFor('the rail says Bo is working', `document.querySelector('[data-crew-room=${js(launch.id)}]')?.textContent.includes('Bo is working')`, 5000).catch(async () => evaluate(`'stale: ' + document.querySelector('[data-crew-room=${js(launch.id)}]')?.textContent`));
    const workingMs = Date.now() - railStart;
    await click('button', 'Room settings');
    const name = '[data-room-dialog] input[aria-label="Name"]';
    await waitFor('Room settings', `!!document.querySelector(${js(name)})`);
    await evaluate(`(() => { const input = document.querySelector(${js(name)}); input.focus(); input.select(); })()`);
    await type('Launch plan');
    await click('[data-room-dialog] button[type="submit"]', 'Save');
    await until('the rename is saved', async () => (await snapshot()).rooms.find((state) => state.room.id === launch.id)?.room.name === 'Launch plan', 5000);
    await waitFor('the dialog closes', `!document.querySelector('[data-room-dialog]')`, 3000);
    const after = roomRuns(await snapshot(), launch.id).filter((run) => queued.some((old) => old.id === run.id));
    const title = await waitFor('the new title', `document.querySelector('#room-title')?.textContent.trim() === 'Launch plan'`, 3000).catch(() => false);
    const renamedAt = Date.now();
    const row = await waitFor('the rail row renamed', `(() => { const text = document.querySelector('[data-crew-room=${js(launch.id)}]')?.textContent.trim() ?? ''; return text.startsWith('Launch plan') ? text : null; })()`, 5000)
      .catch(() => evaluate(`document.querySelector('[data-crew-room=${js(launch.id)}]')?.textContent.trim() ?? ''`));
    const renamedMs = Date.now() - renamedAt;
    release('Bo');
    const answered = await until("both of Bo's runs answer", async () =>
      (await said(launch.id, (message) => queued.some((run) => run.id === message.run_id) && message.text === 'Checked. All good.')).length === 2, 15000).catch(() => false);
    record('renaming in Room settings keeps queued runs',
      working === true && after.length === 2 && after.every(live) && !!title && row.startsWith('Launch plan') && !!answered,
      `working ${js(working)} after ${workingMs}ms; runs after rename ${after.map((run) => run.status).join(',')}; title ${!!title}; rail ${js(row)} in ${renamedMs}ms; both answered ${!!answered}`);
    await settled(launch.id);
  });

  await check('Delete room asks first, its confirm moves focus to Keep and back, and Delete removes the row', async () => {
    await openRoom(launch.id, 'Launch plan');
    await click('button', 'Room settings');
    await waitFor('Room settings', `!!document.querySelector('[data-room-delete]')`);
    await evaluate(`document.querySelector('[data-room-delete]').focus()`);
    await press('Enter');
    const toKeep = await waitFor('focus on Keep', `!!document.querySelector('[data-room-delete-confirm]') && document.activeElement?.hasAttribute('data-room-delete-keep')`, 3000).catch(() => false);
    const asked = await evaluate(`document.querySelector('[data-room-delete-confirm]')?.textContent.includes('Delete this room and its messages? The bruhs stay.') ?? false`);
    const stillThere = !!(await snapshot()).rooms.find((state) => state.room.id === launch.id);
    await press('Enter');
    const back = await waitFor('focus back on Delete room', `!document.querySelector('[data-room-delete-confirm]') && document.activeElement?.hasAttribute('data-room-delete')`, 3000).catch(() => false);
    record('the Delete room confirm moves focus to Keep and back', !!toKeep && !!back, `to Keep ${!!toKeep}; back ${!!back}; focus ${js(await active())}`);
    await press('Enter');
    await waitFor('the confirm again', `!!document.querySelector('[data-room-delete-confirm]')`, 3000);
    await click('[data-room-delete-confirm] button', 'Delete');
    const gone = await until('the room is deleted', async () => !(await snapshot()).rooms.find((state) => state.room.id === launch.id), 5000).catch(() => false);
    const rowGone = await waitFor('the rail row goes', `!document.querySelector('[data-crew-room=${js(launch.id)}]')`, 3000).catch(() => false);
    const notice = await waitFor('the gone notice', `document.querySelector('#room-title')?.textContent.trim() === 'This room no longer exists.'`, 3000).catch(() => false);
    record('Delete room asks first, its confirm moves focus to Keep and back, and Delete removes the row',
      asked && stillThere && !!gone && !!rowGone && !!notice,
      `asked ${asked}; kept until Delete ${stillThere}; deleted ${!!gone}; row gone ${!!rowGone}; notice ${!!notice}`);
  });

  // 11. 390×844 touch: room rows, member checkboxes, cards and Send measure
  // 44px, and the page never scrolls sideways.
  await check('390×844 touch: room rows, member checkboxes, cards and Send measure 44px; no sideways scroll', async () => {
    // Open cards for the phone: an approval and a question waiting on the owner.
    await command({ action: 'post_room', room_id: rooms.cards, text: '@Cy write the plan.' });
    await command({ action: 'post_room', room_id: rooms.cards, text: '@Quinn tidy again.' });
    await until('an open approval and question', async () => {
      const outbox = (await snapshot()).outbox.filter((item) => !item.resolved_at);
      return outbox.some((item) => item.approval && item.agent_id === ids.Cy) && outbox.some((item) => !item.approval && item.agent_id === ids.Quinn);
    }, 10000);
    await viewport(390, 844, true);
    await send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
    await openRoom(rooms.cards, 'Cards');
    // Below sm the sidebar is a drawer over the page; close it with its scrim.
    if (await evaluate(`!!document.querySelector('div.fixed.inset-0.z-30')`)) {
      for (const kind of ['mousePressed', 'mouseReleased']) {
        await send('Input.dispatchMouseEvent', { type: kind, x: 385, y: 600, button: 'left', clickCount: 1 });
      }
      await waitFor('the drawer closes', `!document.querySelector('div.fixed.inset-0.z-30')`, 3000);
    }
    await waitFor('open cards', `document.querySelectorAll('article[aria-label="Approval"] button, article[aria-label="Question"] button').length >= 3`, 5000);
    // The transcript opens on its newest line, as a new line scrolls to it.
    await sleep(500);
    const scroll = await evaluate(`(() => {
      const scroller = document.querySelector('[role="log"][aria-label="Room messages"]').parentElement;
      return { top: Math.round(scroller.scrollTop), height: scroller.scrollHeight, client: scroller.clientHeight };
    })()`);
    record('a room opened at phone size shows its newest line', scroll.height > scroll.client && scroll.height - scroll.client - scroll.top <= 2,
      `scrollTop ${scroll.top}; scrollHeight ${scroll.height}; clientHeight ${scroll.client}`);
    const sizes = (selector) => evaluate(`[...document.querySelectorAll(${js(selector)})].filter((node) => node.getBoundingClientRect().height > 0).map((node) => { const box = node.getBoundingClientRect(); return [Math.round(box.width), Math.round(box.height)]; })`);
    const cardButtons = await sizes('[role="log"] article button');
    const sendSize = await sizes(`${sendButton}`);
    const sideways = await evaluate(`document.scrollingElement.scrollWidth - innerWidth`);
    const coarse = await evaluate(`matchMedia('(pointer: coarse)').matches`);
    await shoot('web-room-390x844-touch');
    await press('B', META | SHIFT);
    await waitFor('the rail on a phone', `[...document.querySelectorAll('[data-crew-room]')].some((row) => row.getBoundingClientRect().height > 0)`, 3000);
    await sleep(300);
    const roomRows = await sizes('[data-crew-room]');
    await click('[data-crew-new-room]');
    await waitFor('the New room dialog', `document.querySelectorAll('[data-room-member]').length === ${names.length}`, 3000);
    const members = await sizes('[data-room-member]');
    const dialogSideways = await evaluate(`document.scrollingElement.scrollWidth - innerWidth`);
    await shoot('web-room-new-390x844-touch');
    await press('Escape');
    await waitFor('the dialog closes', `!document.querySelector('[data-room-dialog]')`, 3000);
    const tall = (list) => list.length > 0 && list.every(([, h]) => h >= 44);
    record('390×844 touch: room rows, member checkboxes, cards and Send measure 44px; no sideways scroll',
      coarse && tall(roomRows) && roomRows.length === 2 && tall(members) && members.length === names.length && tall(cardButtons) && cardButtons.length >= 3 && tall(sendSize) && sideways <= 0 && dialogSideways <= 0,
      `coarse ${coarse}; rooms ${roomRows.map((s) => s[1]).join(',')}; members ${[...new Set(members.map((s) => s[1]))].join(',')}; card buttons ${cardButtons.map((s) => s.join('x')).join(',')}; Send ${sendSize.map((s) => s.join('x')).join(',')}; sideways ${sideways}/${dialogSideways}`);
    await send('Emulation.setTouchEmulationEnabled', { enabled: false });
    await viewport(1280, 800);
  });

  // Deny works from the room too: the open approval ends with Cy's answer.
  await check('Deny on a room approval card resumes Cy without the change', async () => {
    await openRoom(rooms.cards, 'Cards');
    const card = `[data-room-author=${js(ids.Cy)}] article[aria-label="Approval"]`;
    await waitFor('the open approval', `[...document.querySelectorAll(${js(`${card} button`)})].some((button) => button.textContent.trim() === 'Deny')`, 5000);
    const before = (await said(rooms.cards, (message) => message.text === 'Skipped it.')).length;
    const deny = await evaluate(`(() => { const buttons = [...document.querySelectorAll(${js(`${card} button`)})].filter((button) => button.textContent.trim() === 'Deny'); buttons.at(-1).setAttribute('data-verify-deny', ''); return buttons.length; })()`);
    await click('[data-verify-deny]');
    const skipped = await until("Cy's answer", async () => (await said(rooms.cards, (message) => message.text === 'Skipped it.')).length === before + 1, 10000).catch(() => false);
    record('Deny on a room approval card resumes Cy without the change', deny >= 1 && !!skipped, `Deny buttons ${deny}; answered ${!!skipped}`);
  });

  // 12. A long room: Load earlier keeps the reader where they were. Sixty
  // messages go straight into the room's table, the newest Bo's.
  const archive = 'room-archive';
  await check('Load earlier shows the older messages without jumping to the newest line', async () => {
    await command({ action: 'create_room', room: { id: archive, name: 'Archive', members: [ids.Bo], lead: null } });
    const rows = [];
    for (let seq = 1; seq <= 60; seq++) {
      const bo = seq === 60;
      const message = {
        id: `seed-archive-${seq}`, room_id: archive, seq,
        author: bo ? { type: 'bruh', agent_id: ids.Bo } : { type: 'owner' },
        text: bo ? 'Packed:\n\n- **Food** and drinks\n- [the list](https://example.com/list) and [notes](notes/plan.md)\n\n```rust\nfn main() {}\n```' : `Archive message ${seq}`,
        run_id: null, outbox_id: null, failed: false, created_at: Date.now() - (60 - seq) * 1000,
      };
      rows.push(`('${message.id}','${archive}',${seq},NULL,'${JSON.stringify(message).replaceAll("'", "''")}')`);
    }
    const insert = spawnSync(sqlite, [database, '.timeout 5000', `INSERT INTO reactive_room_messages(id,room_id,seq,result_of,payload) VALUES ${rows.join(',')};`], { encoding: 'utf8' });
    if (insert.status !== 0) throw new Error(`seeding the archive failed: ${insert.stderr}`);
    await openRoom(archive, 'Archive');
    const log = '[role="log"][aria-label="Room messages"]';
    await waitFor('the newest page', `document.querySelectorAll(${js(`${log} [data-room-line]`)}).length === 50`, 8000);
    await sleep(400);
    const where = () => evaluate(`(() => {
      const scroller = document.querySelector(${js(log)}).closest('.overflow-y-auto');
      return { top: Math.round(scroller.scrollTop), bottom: Math.round(scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop) };
    })()`);
    const opened = await where();
    await click('button', 'Load earlier');
    await waitFor('the older page', `document.querySelectorAll(${js(`${log} [data-room-line]`)}).length === 60`, 8000);
    await sleep(400);
    const after = await where();
    const first = await evaluate(`document.querySelector(${js(`${log} [data-room-line]`)})?.textContent.trim()`);
    record('Load earlier shows the older messages without jumping to the newest line',
      opened.bottom <= 2 && after.bottom > 200 && first === 'Archive message 1',
      `opened ${js(opened)}; after ${js(after)}; first line ${js(first)}`);
  });

  // 13. Reduced motion: a working bruh's face stops on the room page.
  await check('reduced motion leaves no animation frames on the room page', async () => {
    hold('Fay');
    await openRoom(rooms.cards, 'Cards');
    await post('@Fay try once more.');
    await waitFor('a working line', `!!document.querySelector('[data-room-line="working"]')`, 8000);
    const framesOverASecond = async () => {
      await evaluate('window.__frames = 0');
      await sleep(1000);
      return evaluate('window.__frames');
    };
    const moving = await framesOverASecond();
    await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
    await sleep(300);
    const still = await framesOverASecond();
    await openRoom(rooms.cards, 'Cards');
    await waitFor('a working line after reload', `!!document.querySelector('[data-room-line="working"]')`, 5000);
    await sleep(300);
    const stillAfterLoad = await framesOverASecond();
    record('reduced motion leaves no animation frames on the room page', moving > 0 && still === 0 && stillAfterLoad === 0,
      `frames per second: moving ${moving}, reduced ${still}, reduced after reload ${stillAfterLoad}`);
    await send('Emulation.setEmulatedMedia', { features: [] });
    release('Fay');
    await settled(rooms.cards);
  });

  record('no panics in the page', panics.length === 0, panics.join(' | '));
  record('screenshots saved', screenshots.length > 0 && screenshots.every((file) => fs.statSync(file).size > 0), screenshots.map((file) => path.basename(file)).join(', '));

  exitCode = results.every((result) => result.pass) ? 0 : 1;
  fs.writeFileSync(path.join(runDir, 'results.json'), JSON.stringify({ when: new Date().toISOString(), base, results, screenshots }, null, 2));
  console.log(`Screenshots: ${shots}`);
  const failed = results.filter((result) => !result.pass).length;
  console.log(`${results.length - failed}/${results.length} checks passed.`);
  console.log(exitCode === 0 ? 'All room checks passed.' : `${failed} room check(s) failed.`);
} catch (error) {
  console.error(`FAIL ${error.stack ?? error}`);
  fs.writeFileSync(path.join(runDir, 'results.json'), JSON.stringify({ when: new Date().toISOString(), results, error: String(error) }, null, 2));
} finally {
  await cleanup();
}
process.exit(exitCode);
