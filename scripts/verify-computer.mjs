// Acceptance run for a bruh's Computer on the web (docs/crew.md "Computer";
// PARITY-18 in docs/web-desktop-parity-goal-2026-11-15.md): the Settings
// switch and its reasons, the Computer tab and its live view, a bruh's
// browser steps through the gate (Full, Ask first, refused scripts, this
// computer), Take control and Hand back with owner input and paste, the
// profile's cookie across a close, isolation between bruhs, the live view's
// origin rule, downloads, a hidden tab past the ack deadline, Stop all,
// Computer off, Delete agent and its folder, keyboard, phone-size touch and
// reduced motion. Any panic in the page fails the run. Every check prints ok
// or FAIL (or skip, with the reason, for what this stand-in can't drive);
// the exit status is 0 only when all pass.
//
//   cargo build -p bruhs-server
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43051 trunk build --dist "$TMPDIR/bruhs-computer-dist")
//   node scripts/verify-computer.mjs "$PWD" "$TMPDIR/bruhs-computer-dist" 43051
//
// The bundle must be built for the port the check uses. bruhs-server runs
// with a disposable HOME and BRUHS_REACTIVE_DB, a capture URL nothing listens
// on and, first on its PATH, a stand-in `claude`, so no provider is called
// and no real data is touched. The stand-in reads `--mcp-config=` from its
// arguments, starts the `bruhs_workbench` helper it names with that env, and
// runs each `do: {json}` line of its prompt as a `local_browser` call: first
// a `control_request` `can_use_tool` for mcp__bruhs_workbench__local_browser,
// then, once the server allows it, the helper's `tools/call`. Every tool
// reply goes into its `result`. A file `hold-<name>` beside it keeps that
// bruh working until it is removed.
//
// Fixtures. A bruh's browser goes through its network guard, which refuses
// this computer: loopback and every address on this computer's interfaces
// (docs/crew.md "Network"). A page server here can't be reached at all, so
// the fixture pages live on a name that never resolves,
// http://fixture.bruhs-verify.example, and this script answers them itself
// over the bruh browser's own DevTools port (read from DevToolsActivePort in
// its profile) with Fetch.requestPaused / Fetch.fulfillRequest. Requests to
// any other host still go through the guard, which is what the trap checks.
// A real page server on this computer's first non-internal IPv4 address
// shows the guard refuses it (the script stops when there is none), and a
// counting server on 127.0.0.1 shows a page can't reach loopback.
//
// Headless Chrome (BRUHS_VERIFY_CHROME, default the macOS app) runs on
// BRUHS_VERIFY_CDP_PORT (default 9451) with its own profile. Screenshots,
// logs and results go to $TMPDIR/bruhs-computer-verify/, cleared on each run.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import { networkInterfaces, tmpdir } from 'node:os';
import path from 'node:path';

const root = path.resolve(process.argv[2] ?? '');
const dist = path.resolve(process.argv[3] ?? '');
const appPort = Number(process.argv[4] ?? 43051);
const serverBin = path.join(root, 'target/debug/bruhs-server');
for (const [label, file] of [['bruhs-server', serverBin], ['web bundle', path.join(dist, 'index.html')]]) {
  if (!fs.existsSync(file)) {
    console.error(`${label} not found at ${file}; see the usage at the top of this script.`);
    process.exit(2);
  }
}
const lanAddress = Object.values(networkInterfaces()).flat().find((entry) => entry?.family === 'IPv4' && !entry.internal)?.address;
if (!lanAddress) {
  console.error('This computer has no non-internal IPv4 address, so the LAN check has nothing to try. Connect to a network and run again.');
  process.exit(2);
}
const cdpPort = Number(process.env.BRUHS_VERIFY_CDP_PORT ?? 9451);
const chromeBin = process.env.BRUHS_VERIFY_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const runDir = path.join(fs.realpathSync(tmpdir()), 'bruhs-computer-verify');
fs.rmSync(runDir, { recursive: true, force: true });
const home = path.join(runDir, 'home');
const bin = path.join(runDir, 'bin');
const shots = path.join(runDir, 'shots');
for (const dir of [home, bin, shots, path.join(runDir, 'chrome')]) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
const database = path.join(runDir, 'reactive.sqlite3');
// The reactive runtime's home is the database's folder; each bruh's tree is
// bruhs/<id> in it.
const tree = (agentId) => path.join(path.dirname(database), 'bruhs', agentId);
console.log(`Run directory: ${runDir}`);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const js = JSON.stringify;

// ── The stand-in claude ──────────────────────────────────────────────────────
const standIn = path.join(bin, 'claude');
fs.writeFileSync(standIn, `#!${process.execPath}
const fs = require('fs');
const path = require('path');
const readline = require('readline');
const { spawn } = require('child_process');
const args = process.argv.slice(2);
if (args[0] === 'auth') {
  console.log(JSON.stringify({ loggedIn: true, authMethod: 'claude.ai', email: 'verify@example.invalid' }));
  process.exit(0);
}
if (args[0] === '--version' || args[0] === '-v') {
  console.log('2.1.0 (Claude Code)');
  process.exit(0);
}
const TOOL = 'mcp__bruhs_workbench__local_browser';
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const out = (event) => process.stdout.write(JSON.stringify(event) + '\\n');
const log = (file, entry) => fs.appendFileSync(path.join(__dirname, file), JSON.stringify({ at: Date.now(), ...entry }) + '\\n');
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
// The bruhs_workbench helper the run's --mcp-config names, over stdio.
function helper(config) {
  const child = spawn(config.command, config.args ?? [], { env: { ...process.env, ...(config.env ?? {}) }, stdio: ['pipe', 'pipe', 'ignore'] });
  process.on('exit', () => child.kill());
  const replies = new Map();
  let buffer = '';
  child.stdout.on('data', (chunk) => {
    buffer += chunk;
    let at;
    while ((at = buffer.indexOf('\\n')) >= 0) {
      const line = buffer.slice(0, at);
      buffer = buffer.slice(at + 1);
      try {
        const message = JSON.parse(line);
        replies.get(message.id)?.(message);
      } catch {}
    }
  });
  let id = 0;
  const call = (method, params) => new Promise((resolve) => {
    id += 1;
    replies.set(id, resolve);
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\\n');
  });
  return { call, notify: (method) => child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method }) + '\\n') };
}
(async () => {
  const prompt = text(await next());
  const name = (prompt.match(/Your name is ([^.\\n]+)\\./) ?? [])[1] ?? 'Unknown';
  const mcp = args.find((arg) => arg.startsWith('--mcp-config='))?.slice('--mcp-config='.length);
  const server = mcp ? JSON.parse(mcp).mcpServers?.bruhs_workbench : undefined;
  log('calls.log', { name, prompt, browser: !!server });
  const session = 'verify-computer-' + name.toLowerCase();
  out({ type: 'system', subtype: 'init', session_id: session, model: 'claude-verify', tools: [], mcp_servers: server ? [{ name: 'bruhs_workbench', status: 'connected' }] : [] });
  while (fs.existsSync(path.join(__dirname, 'hold-' + name))) await sleep(150);
  const steps = [...prompt.matchAll(/^do: (.+)$/gm)].map((match) => JSON.parse(match[1]));
  const replies = [];
  if (steps.length > 0 && !server) replies.push('No browser tool in this run.');
  let tools;
  if (steps.length > 0 && server) {
    tools = helper(server);
    await tools.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'verify-claude', version: '1' } });
    tools.notify('notifications/initialized');
    await tools.call('tools/list', {});
  }
  for (const [index, command] of (server ? steps : []).entries()) {
    const requestId = name + '-' + Date.now() + '-' + index;
    const input = { command };
    log('steps.log', { name, phase: 'ask', command });
    out({ type: 'control_request', request_id: requestId, request: { subtype: 'can_use_tool', tool_name: TOOL, input, tool_use_id: 'toolu_' + requestId } });
    let decision;
    while (!decision) {
      const line = await next();
      try {
        const message = JSON.parse(line);
        if (message.type === 'control_response' && message.response?.request_id === requestId) decision = message.response.response;
      } catch {}
    }
    if (decision.behavior !== 'allow') {
      log('steps.log', { name, phase: 'denied', command, message: decision.message });
      replies.push('Denied: ' + decision.message);
      continue;
    }
    const reply = await tools.call('tools/call', { name: 'local_browser', arguments: decision.updatedInput ?? input });
    const said = (reply.result?.content ?? []).filter((part) => part.type === 'text').map((part) => part.text).join('\\n');
    log('steps.log', { name, phase: 'reply', command, error: !!reply.result?.isError, reply: said.slice(0, 2000) });
    replies.push((reply.result?.isError ? 'Error: ' : '') + said);
  }
  const answer = replies.length > 0 ? replies.join('\\n\\n') : 'Done.';
  out({ type: 'result', subtype: 'success', is_error: false, result: answer, session_id: session, duration_ms: 1 });
  process.exit(0);
})();
`, { mode: 0o755 });
const hold = (name) => fs.writeFileSync(path.join(bin, `hold-${name}`), '');
const release = (name) => fs.rmSync(path.join(bin, `hold-${name}`), { force: true });
const readLog = (file) => {
  try {
    return fs.readFileSync(path.join(bin, file), 'utf8').split('\n').filter(Boolean).map((line) => JSON.parse(line));
  } catch {
    return [];
  }
};

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
const servers = new Set();
const sockets = new Set();
async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once('exit', resolve));
  child.kill('SIGTERM');
  if (await Promise.race([exited.then(() => true), sleep(8000).then(() => false)]) === false) child.kill('SIGKILL');
  children.delete(child);
}
async function cleanup() {
  for (const file of fs.readdirSync(bin)) if (file.startsWith('hold-')) fs.rmSync(path.join(bin, file), { force: true });
  for (const socket of sockets) try { socket.close(); } catch {}
  for (const child of [...children].reverse()) await stopChild(child);
  for (const server of servers) server.close();
  spawnSync('/usr/bin/pkill', ['-f', standIn]);
  // A bruh browser the server didn't get to close: its profile is in this run.
  spawnSync('/usr/bin/pkill', ['-f', `--user-data-dir=${path.dirname(database)}/bruhs/`]);
}
process.on('SIGINT', () => cleanup().then(() => process.exit(130)));

const results = [];
const record = (name, pass, detail = '') => {
  results.push({ name, pass: !!pass, detail: String(detail).slice(0, 800) });
  console.log(`${pass ? 'ok  ' : 'FAIL'} ${name}${detail ? ` (${String(detail).slice(0, 500)})` : ''}`);
  return !!pass;
};
const skip = (name, reason) => {
  results.push({ name, pass: true, skipped: true, detail: reason });
  console.log(`skip ${name} (${reason})`);
};
// A check that throws fails on its own; the run goes on to the next one.
const check = async (name, body) => {
  try {
    await body();
  } catch (error) {
    record(name, false, error?.message ?? String(error));
  }
};

// ── Fixture servers on this computer ─────────────────────────────────────────
// Loopback: counts every connection; a bruh's page must never reach it.
let loopbackConnections = 0;
const counting = net.createServer((socket) => {
  loopbackConnections += 1;
  socket.once('data', () => socket.end('HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok'));
  socket.on('error', () => {});
});
await new Promise((resolve) => counting.listen(0, '127.0.0.1', resolve));
servers.add(counting);
const countingPort = counting.address().port;
// This computer's LAN address: the guard refuses it too.
let lanHits = 0;
const lan = http.createServer((request, response) => {
  lanHits += 1;
  response.end('<title>LAN fixture</title>reached');
});
await new Promise((resolve) => lan.listen(0, lanAddress, resolve));
servers.add(lan);
const lanPort = lan.address().port;

// The fixture host, answered over each bruh browser's DevTools port.
const FIXTURE = 'http://fixture.bruhs-verify.example';
const DOWNLOAD_NAME = 'bruhs-download-probe.txt';
const fixtureLog = [];
const logged = (agent, kind) => fixtureLog.filter((entry) => entry.agent === agent && entry.kind === kind);
const pages = {
  '/form': () => `<!doctype html><meta charset=utf-8><title>Bruhs fixture form</title>
<style>body{margin:0;font:16px sans-serif;background:#fff}#press{position:absolute;left:100px;top:120px;width:300px;height:160px}#q{position:absolute;left:100px;top:320px;width:300px;height:40px}#download{position:absolute;left:100px;top:400px}#cookie{position:absolute;left:100px;top:40px}</style>
<p id=cookie></p><button id=press>Press</button><input id=q autocomplete=off aria-label=Query><a id=download href="/download">Download the probe</a>
<script>
const before = document.cookie || 'none';
document.getElementById('cookie').textContent = 'Cookie: ' + before;
const log = (kind, value) => fetch('/log?' + new URLSearchParams({ kind, value }), { method: 'POST' }).catch(() => {});
document.getElementById('press').addEventListener('click', (event) => log('click', String(event.isTrusted)));
document.getElementById('q').addEventListener('input', (event) => log('typed', event.target.value));
</script>`,
  '/login': () => `<!doctype html><title>Bruhs fixture login</title><p>Signed in.</p>`,
  '/trap': () => `<!doctype html><title>Bruhs fixture trap</title><img src="http://127.0.0.1:${countingPort}/pixel.png" alt="">
<script>fetch('http://127.0.0.1:${countingPort}/fetch').then(() => 'reached', (error) => 'failed').then((outcome) => fetch('/log?' + new URLSearchParams({ kind: 'trap', value: outcome }), { method: 'POST' }));</script>`,
  '/socket': (query) => `<!doctype html><title>Bruhs fixture socket</title><script>
const log = (kind, value) => fetch('/log?' + new URLSearchParams({ kind, value }), { method: 'POST' }).catch(() => {});
try {
  const socket = new WebSocket('ws://127.0.0.1:${Number(query.get('port'))}/api/local-tools/browser/' + ${js(query.get('session') ?? '')} + '/screencast');
  socket.onopen = () => log('socket', 'open');
  socket.onerror = () => log('socket', 'error');
} catch (error) { log('socket', 'threw ' + error); }
fetch('http://127.0.0.1:${Number(query.get('port'))}/api/reactive/agents/' + ${js(query.get('agent') ?? '')} + '/computer').then((response) => response.text()).then((body) => log('state', 'read ' + body.slice(0, 300)), (error) => log('state', 'failed ' + error));
</script>`,
  '/ticker': () => `<!doctype html><title>Bruhs fixture ticker</title><style>body{margin:0;font:120px monospace}</style><div id=n>0</div>
<script>let n = 0; setInterval(() => { n += 1; document.getElementById('n').textContent = n; document.body.style.background = n % 2 ? '#fff' : '#eee'; }, 100);</script>`,
};
const loginCookie = `signed-in-${Date.now().toString(36)}`;
const bruhBrowsers = new Map();
async function fulfill(agent, cdp, sessionId, params) {
  const url = new URL(params.request.url);
  const header = (name) => Object.entries(params.request.headers ?? {}).find(([key]) => key.toLowerCase() === name)?.[1] ?? '';
  const entry = { agent, at: Date.now(), path: url.pathname, kind: 'page', cookie: header('cookie') };
  let status = 200;
  let headers = [{ name: 'content-type', value: 'text/html; charset=utf-8' }, { name: 'cache-control', value: 'no-store' }];
  let body = '';
  if (url.pathname === '/log') {
    entry.kind = url.searchParams.get('kind');
    entry.value = url.searchParams.get('value');
    status = 204;
  } else if (url.pathname === '/download') {
    entry.kind = 'download';
    headers = [{ name: 'content-type', value: 'text/plain' }, { name: 'content-disposition', value: `attachment; filename="${DOWNLOAD_NAME}"` }];
    body = 'A bruh browser must not save this.';
  } else if (pages[url.pathname]) {
    body = pages[url.pathname](url.searchParams);
    // A login: a persistent cookie, as a site sets one.
    if (url.pathname === '/login') headers.push({ name: 'set-cookie', value: `bruh_cookie=${loginCookie}; Max-Age=3600; Path=/` });
  } else {
    status = 404;
    body = 'Not here.';
  }
  fixtureLog.push(entry);
  await cdp.send('Fetch.fulfillRequest', { requestId: params.requestId, responseCode: status, responseHeaders: headers, body: Buffer.from(body).toString('base64') }, sessionId).catch(() => {});
}
// Attach to a bruh's running Chrome and answer the fixture host in each of
// its pages. Called after every launch; a relaunch writes a new port.
async function serveFixture(agent, timeout = 10000) {
  const file = path.join(tree(agent), 'browser', 'DevToolsActivePort');
  const deadline = Date.now() + timeout;
  for (;;) {
    try {
      const [port, browserPath] = fs.readFileSync(file, 'utf8').trim().split('\n');
      const current = bruhBrowsers.get(agent);
      if (current && current.port === port && current.socket.readyState === WebSocket.OPEN) return;
      const version = await (await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) })).json();
      if (!version.webSocketDebuggerUrl.endsWith(browserPath)) throw new Error('stale DevToolsActivePort');
      current?.socket.close();
      const cdp = await connect(version.webSocketDebuggerUrl);
      sockets.add(cdp.socket);
      bruhBrowsers.set(agent, { port, socket: cdp.socket });
      const attached = new Set();
      const attach = async (targetId) => {
        if (attached.has(targetId)) return;
        attached.add(targetId);
        const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
        await cdp.send('Fetch.enable', { patterns: [{ urlPattern: `${FIXTURE}/*`, requestStage: 'Request' }] }, sessionId);
      };
      cdp.listeners.add((message) => {
        if (message.method === 'Fetch.requestPaused') fulfill(agent, cdp, message.sessionId, message.params);
        if (message.method === 'Target.targetCreated' && message.params.targetInfo.type === 'page') attach(message.params.targetInfo.targetId).catch(() => {});
      });
      await cdp.send('Target.setDiscoverTargets', { discover: true });
      for (const target of (await cdp.send('Target.getTargets')).targetInfos) {
        if (target.type === 'page') await attach(target.targetId);
      }
      return;
    } catch (error) {
      if (Date.now() > deadline) throw new Error(`no DevTools for ${agent}'s browser: ${error.message}`);
      await sleep(200);
    }
  }
}

// A CDP connection: `send(method, params, sessionId)` and event listeners.
async function connect(url) {
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, { once: true });
    socket.addEventListener('error', () => reject(new Error(`can't connect to ${url}`)), { once: true });
  });
  let nextId = 1;
  const waiting = new Map();
  const listeners = new Set();
  socket.addEventListener('message', (event) => {
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
  socket.addEventListener('close', () => {
    for (const { reject } of waiting.values()) reject(new Error('the DevTools connection closed'));
    waiting.clear();
  });
  const send = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      if (socket.readyState !== WebSocket.OPEN) return reject(new Error('the DevTools connection is closed'));
      const id = nextId++;
      waiting.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  return { socket, send, listeners };
}

// A raw WebSocket upgrade, so the Origin header is ours to set.
const upgrade = (route, origin) => new Promise((resolve) => {
  const request = http.request({
    host: '127.0.0.1', port: appPort, path: route,
    headers: { connection: 'Upgrade', upgrade: 'websocket', 'sec-websocket-version': '13', 'sec-websocket-key': Buffer.from('bruhs-verify-key').toString('base64'), ...(origin ? { origin } : {}) },
  });
  request.on('upgrade', (response, socket) => {
    socket.destroy();
    resolve(response.statusCode);
  });
  request.on('response', (response) => {
    response.resume();
    resolve(response.statusCode);
  });
  request.on('error', (error) => resolve(`error ${error.message}`));
  request.end();
});

// Every file under `dir` whose name includes `name`, without following links.
const findNamed = (dir, name) => {
  const found = [];
  const walk = (folder) => {
    let entries = [];
    try {
      entries = fs.readdirSync(folder, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(folder, entry.name);
      if (entry.name.includes(name)) found.push(full);
      if (entry.isDirectory()) walk(full);
    }
  };
  walk(dir);
  return found;
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
  const computer = (id) => api('GET', `/api/reactive/agents/${id}/computer`);
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
  const runsOf = async (agentId) => (await snapshot()).runs.filter((run) => run.agent_id === agentId).sort((a, b) => a.created_at - b.created_at);
  const live = (run) => run.status === 'queued' || run.status === 'running';
  const settled = (agentId, timeout = 20000) => until(`${agentId} settles`, async () => !(await runsOf(agentId)).some(live), timeout);
  // The run a new message started: the newest one not in `before`.
  const newRun = (agentId, before, timeout = 8000) =>
    until(`a new run for ${agentId}`, async () => (await runsOf(agentId)).find((run) => !before.includes(run.id)), timeout);
  const finished = (runId, timeout = 30000) =>
    until(`run ${runId} ends`, async () => {
      const run = (await snapshot()).runs.find((candidate) => candidate.id === runId);
      return run && !live(run) ? run : null;
    }, timeout);
  const steps = (name) => readLog('steps.log').filter((entry) => entry.name === name);
  const doLines = (...commands) => commands.map((step) => `do: ${js(step)}`).join('\n');
  // A chat sent by the service, as `message_bruh` does: the run's reply.
  const ask = async (agentId, message) => {
    const before = (await runsOf(agentId)).map((run) => run.id);
    await command({ action: 'chat', agent_id: agentId, message });
    return finished((await newRun(agentId, before)).id);
  };

  // ── Seed: four bruhs, every Computer off ────────────────────────────────────
  const ids = { Pix: 'computer-pix', Ash: 'computer-ash', Rex: 'computer-rex', Cody: 'computer-cody' };
  const seed = [
    ['Pix', 'claude-sonnet-4-6', 'full', ['left', 'blueberry', 'bruh']],
    ['Ash', 'claude-sonnet-4-6', 'ask', ['front', 'pandan', 'bruh']],
    ['Rex', 'claude-sonnet-4-6', 'read_only', ['right', 'mangosteen', 'longan']],
    ['Cody', 'gpt-5-codex', 'full', ['front', 'bruh', 'dragonfruit']],
  ];
  for (const [name, model, level, [look, face, mouth]] of seed) {
    await command({
      action: 'create_bruh',
      agent: { id: ids[name], name, instructions: `${name} uses its own browser.`, model, project_path: '', enabled: true },
      look: { look, face, mouth },
      access: { level, send_as_me: false, can_deliver: false },
    });
  }
  // Pix's queued run: one held run, and a second message behind it.
  hold('Pix');
  await command({ action: 'chat', agent_id: ids.Pix, message: 'Start with the notes.' });
  await until("Pix's first run is running", async () => (await runsOf(ids.Pix)).some((run) => run.status === 'running'));
  await command({ action: 'chat', agent_id: ids.Pix, message: 'Then the list.' });
  const queued = await until("Pix's second run queues", async () => (await runsOf(ids.Pix)).find((run) => run.status === 'queued'));

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
  const page = await connect(targets.find((t) => t.type === 'page').webSocketDebuggerUrl);
  sockets.add(page.socket);
  const { send, listeners } = page;
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
  // Click the centre of the first element matching `selector` (optionally
  // the one whose text is `text`), or of the button holding it, after
  // checking nothing covers it. Controls that follow the Computer's state
  // appear once the tab has read it, so it waits up to 3 s for one.
  const click = async (selector, text) => {
    const find = () => evaluate(`(() => {
      const all = [...document.querySelectorAll(${js(selector)})];
      const found = ${text === undefined ? 'all[0]' : `all.find((node) => node.textContent.trim() === ${js(text)})`};
      const element = found?.closest('button') ?? found;
      if (!element) return null;
      element.scrollIntoView({ block: 'center' });
      const box = element.getBoundingClientRect();
      const x = box.left + box.width / 2, y = box.top + box.height / 2;
      const hit = document.elementFromPoint(x, y);
      const covered = !(hit && (hit === element || element.contains(hit)));
      return { x, y, covered, by: covered && hit ? hit.tagName + '.' + [...hit.classList].join('.') + ' ' + hit.textContent.trim().slice(0, 40) : null, disabled: !!element.disabled };
    })()`);
    let point = await find();
    for (const deadline = Date.now() + 3000; (!point || point.covered || point.disabled) && Date.now() < deadline;) {
      await sleep(100);
      point = await find();
    }
    const label = `${selector}${text === undefined ? '' : ` "${text}"`}`;
    if (!point) throw new Error(`Nothing matches ${label}`);
    if (point.covered) throw new Error(`${label} is covered by ${point.by}`);
    if (point.disabled) throw new Error(`${label} is disabled`);
    for (const type of ['mousePressed', 'mouseReleased']) {
      await send('Input.dispatchMouseEvent', { type, x: point.x, y: point.y, button: 'left', clickCount: 1 });
    }
  };
  const META = 4;
  const keys = {
    Tab: ['Tab', 9], Enter: ['Enter', 13], Escape: ['Escape', 27], F6: ['F6', 117], '.': ['Period', 190], v: ['KeyV', 86],
    h: ['KeyH', 72], i: ['KeyI', 73],
  };
  const press = async (key, modifiers = 0, extra = {}) => {
    const [code, keyCode] = keys[key];
    const text = modifiers === 0 && key.length === 1 ? { text: key, unmodifiedText: key } : {};
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode: keyCode, modifiers, ...text, ...extra });
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
    return element ? { tag: element.tagName, label: element.getAttribute('aria-label'), live: element.hasAttribute('data-computer-live'), address: element.hasAttribute('data-computer-address'), text: element.textContent.trim().slice(0, 40) } : null;
  })()`);

  // Any Rust panic in the page is a failure, whatever check was running.
  const panics = [];
  // The commands the page sends, and its live view sockets' frames.
  const sent = [];
  const liveSockets = new Map();
  listeners.add((message) => {
    if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
      const text = message.params.args.map((arg) => arg.value ?? arg.description ?? '').join(' ');
      if (text.includes('panicked')) panics.push(text.slice(0, 300));
    }
    if (message.method === 'Network.requestWillBeSent') {
      const { request } = message.params;
      if (request.method === 'POST' && request.url.endsWith('/api/reactive/commands')) {
        try {
          sent.push({ at: Date.now(), ...JSON.parse(request.postData ?? 'null') });
        } catch {}
      }
    }
    if (message.method === 'Network.webSocketCreated' && message.params.url.includes('/screencast')) {
      liveSockets.set(message.params.requestId, { url: message.params.url, frames: [], acks: 0, texts: [], closed: false });
    }
    const socket = liveSockets.get(message.params?.requestId);
    if (socket && message.method === 'Network.webSocketFrameReceived') {
      if (message.params.response.opcode === 2) socket.frames.push(Date.now());
      else socket.texts.push(message.params.response.payloadData.slice(0, 300));
    }
    if (socket && message.method === 'Network.webSocketFrameSent' && message.params.response.payloadData === 'frame_received') socket.acks += 1;
    if (socket && message.method === 'Network.webSocketClosed') socket.closed = true;
  });
  const framesSince = (since) => [...liveSockets.values()].reduce((count, socket) => count + socket.frames.filter((at) => at >= since).length, 0);
  const commandsSince = (since, action) => sent.filter((entry) => entry.at >= since && entry.action === action);
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Network.enable');
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
  const openBruh = async (name, tab) => {
    await openApp(`#/bruhs/${ids[name]}`);
    await waitFor(`${name}'s page`, `document.querySelector('#bruh-title')?.textContent.trim() === ${js(name)}`);
    if (tab) await showTab(tab);
  };
  const showTab = async (tab) => {
    await click('[role="tab"]', tab);
    if (tab === 'Settings') await waitFor('the Settings form', `!!document.querySelector('form[aria-label="Bruh settings"] [data-bruh-computer]')`);
    if (tab === 'Computer') await waitFor('the Computer tab', `!!document.querySelector('[data-bruh-computer] [data-computer-status]')`);
    if (tab === 'Chat') await waitFor('the chat', `!!document.querySelector('textarea[aria-label="Message"]')`);
  };
  const statusLine = () => evaluate(`document.querySelector('[data-computer-status]')?.textContent.trim() ?? null`);
  const waitStatus = (line, timeout = 8000) => waitFor(`the status "${line}"`, `document.querySelector('[data-computer-status]')?.textContent.trim() === ${js(line)}`, timeout);
  const hasFrame = () => evaluate(`(() => { const img = document.querySelector('[data-computer-live]'); return !!img && img.naturalWidth > 0 && img.getAttribute('src')?.startsWith('blob:'); })()`);
  // The page point (in the 1280 by 800 view) at a place in the live view.
  const viewPoint = async (x, y) => evaluate(`(() => {
    const img = document.querySelector('[data-computer-live]');
    img.scrollIntoView({ block: 'center' });
    const box = img.getBoundingClientRect();
    const scale = Math.min(box.width / 1280, box.height / 800);
    const left = box.left + (box.width - 1280 * scale) / 2, top = box.top + (box.height - 800 * scale) / 2;
    return { x: left + ${x} * scale, y: top + ${y} * scale };
  })()`);
  const clickView = async (x, y) => {
    const point = await viewPoint(x, y);
    for (const kind of ['mousePressed', 'mouseReleased']) {
      await send('Input.dispatchMouseEvent', { type: kind, x: point.x, y: point.y, button: 'left', buttons: kind === 'mousePressed' ? 1 : 0, clickCount: 1 });
      await sleep(60);
    }
  };
  const controlButton = '[data-computer-control]';
  const browserButton = '[data-computer-browser]';
  const lines = {
    closed: (name) => `The browser is closed. ${name} opens it on its next step, or you can open it now.`,
    has: (name) => `${name} has the browser.`,
    held: (name) => `You have control. ${name} waits until you hand it back.`,
    off: 'Computer is off. Turn it on in Settings.',
  };
  const takeControl = async () => {
    await click(controlButton, 'Take control');
    await waitStatus(lines.held('Pix'));
  };
  const handBack = async () => {
    await click(controlButton, 'Hand back');
    await waitStatus(lines.has('Pix'));
  };
  const chatInPage = async (message) => {
    await evaluate(`document.querySelector('textarea[aria-label="Message"]').focus()`);
    await type(message);
    await click('button', 'Send');
  };
  const NOT_FOR_BRUHS = "This browser action isn't available to bruhs. Take a snapshot to read the page.";
  const LOCAL_DENIAL = "Bruhs can't reach services on this computer.";

  // 1. Settings: Computer is off for every bruh; Rex and Cody say why.
  await check('Settings: Computer is off by default; Rex and Cody say why it is unavailable', async () => {
    const stored = (await snapshot()).bruhs.map((state) => `${state.name}:${state.computer}`);
    const switchState = async (name) => {
      await openBruh(name, 'Settings');
      return evaluate(`(() => {
        const toggle = document.querySelector('[data-bruh-computer]');
        return { checked: toggle.getAttribute('aria-checked'), disabled: toggle.disabled, reason: document.querySelector('[data-bruh-computer-reason]')?.textContent.trim() ?? null, hint: document.querySelector('[data-bruh-computer-hint]')?.textContent.trim() ?? null };
      })()`);
    };
    const pix = await switchState('Pix');
    await shoot('web-settings-pix-1280x800');
    const rex = await switchState('Rex');
    const cody = await switchState('Cody');
    record('Settings: Computer is off by default; Rex and Cody say why it is unavailable',
      stored.every((entry) => entry.endsWith(':false')) && pix.checked === 'false' && !pix.disabled && pix.reason === null
        && rex.checked === 'false' && rex.disabled && rex.reason === 'Computer needs Ask first or Full access.'
        && cody.checked === 'false' && cody.disabled && cody.reason === 'Computer needs a Claude model.',
      `stored ${stored.join(',')}; Pix ${js(pix)}; Rex ${js(rex)}; Cody ${js(cody)}`);
  });

  // 2. Turning Computer on for Pix sends one set_computer and leaves the
  // queued run queued.
  await check("turning Computer on for Pix sends one set_computer and leaves Pix's queued run queued", async () => {
    await openBruh('Pix', 'Settings');
    const since = Date.now();
    await click('[data-bruh-computer]');
    await waitFor('the switch is on', `document.querySelector('[data-bruh-computer]').getAttribute('aria-checked') === 'true'`, 3000);
    await click('form[aria-label="Bruh settings"] button', 'Save');
    await until('Computer is on for Pix', async () => (await snapshot()).bruhs.find((state) => state.agent_id === ids.Pix)?.computer === true, 5000);
    await sleep(1000);
    const setComputer = commandsSince(since, 'set_computer');
    const others = sent.filter((entry) => entry.at >= since && entry.action !== 'set_computer').map((entry) => entry.action);
    const runs = await runsOf(ids.Pix);
    const stillQueued = runs.find((run) => run.id === queued.id)?.status;
    record("turning Computer on for Pix sends one set_computer and leaves Pix's queued run queued",
      setComputer.length === 1 && setComputer[0].agent_id === ids.Pix && setComputer[0].on === true && others.length === 0 && stillQueued === 'queued',
      `set_computer ${js(setComputer.map(({ at, ...rest }) => rest))}; other commands ${others.join(',') || 'none'}; queued run ${stillQueued}; runs ${runs.map((run) => run.status).join(',')}`);
  });
  release('Pix');
  await settled(ids.Pix);
  // Ash gets its Computer here; check 5 opens it.
  await command({ action: 'set_computer', agent_id: ids.Ash, on: true });

  // 3. The closed line, then Open browser shows frames within 5 s.
  await check('the Computer tab shows the closed line; Open browser shows frames within 5 s and "Pix has the browser."', async () => {
    await openBruh('Pix', 'Computer');
    const closed = await waitStatus(lines.closed('Pix')).then(() => true, async () => statusLine());
    await shoot('web-computer-closed-1280x800');
    const clicked = Date.now();
    await click(browserButton, 'Open browser');
    const framed = await waitFor('a frame in the live view', `(() => { const img = document.querySelector('[data-computer-live]'); return !!img && img.naturalWidth > 0; })()`, 5000).then(() => Date.now() - clicked, () => null);
    const has = await waitStatus(lines.has('Pix'), 3000).then(() => true, async () => statusLine());
    await serveFixture(ids.Pix);
    await shoot('web-computer-open-1280x800');
    await viewport(640, 480);
    await shoot('web-computer-open-640x480');
    await viewport(1280, 800);
    record('the Computer tab shows the closed line; Open browser shows frames within 5 s and "Pix has the browser."',
      closed === true && framed !== null && framed <= 5000 && framesSince(clicked) > 0 && has === true,
      `closed line ${js(closed)}; first frame after ${framed} ms; frames ${framesSince(clicked)}; status ${js(has)}`);
  });

  // 4. Pix navigates and reads the page.
  await check("Pix's navigate and snapshot: the reply holds the page title and the tab's address shows the URL", async () => {
    await openBruh('Pix', 'Chat');
    const before = (await runsOf(ids.Pix)).map((run) => run.id);
    await chatInPage(doLines({ action: 'navigate', url: `${FIXTURE}/form` }, { action: 'snapshot' }));
    const run = await finished((await newRun(ids.Pix, before)).id);
    const shown = await waitFor("Pix's reply in the chat", `document.querySelector('[role="log"][aria-label="Conversation"]')?.textContent.includes('Bruhs fixture form')`, 8000)
      .catch(() => evaluate(`'missing: ' + (document.querySelector('[role="log"][aria-label="Conversation"]')?.textContent.slice(-400) ?? 'no log')`));
    await showTab('Computer');
    const address = await waitFor('the address shows the page', `(() => { const value = document.querySelector('[data-computer-address]')?.value; return value === ${js(`${FIXTURE}/form`)} ? value : null; })()`, 8000)
      .catch(() => evaluate(`document.querySelector('[data-computer-address]')?.value ?? null`));
    record("Pix's navigate and snapshot: the reply holds the page title and the tab's address shows the URL",
      run.status === 'completed' && run.result?.includes('"title":"Bruhs fixture form"') && shown === true && address === `${FIXTURE}/form` && logged(ids.Pix, 'page').some((entry) => entry.path === '/form'),
      `run ${run.status}; title in the reply ${run.result?.includes('"title":"Bruhs fixture form"')}; in chat ${js(shown)}; address ${js(address)}`);
  });

  // 5. Ash at Ask first: an approval card names the page; Approve runs it.
  await check('Ash\'s navigate shows an approval card "Open …/form", and Approve runs it', async () => {
    await api('POST', `/api/reactive/agents/${ids.Ash}/computer`, { action: 'open' });
    await serveFixture(ids.Ash);
    await openBruh('Ash', 'Chat');
    const before = (await runsOf(ids.Ash)).map((run) => run.id);
    await chatInPage(doLines({ action: 'navigate', url: `${FIXTURE}/form` }, { action: 'snapshot' }));
    const started = await newRun(ids.Ash, before);
    const summary = `Open ${FIXTURE}/form`;
    const card = await waitFor('the approval card', `(() => {
      const card = [...document.querySelectorAll('article[aria-label="Approval"]')].find((node) => [...node.querySelectorAll('button')].some((button) => button.textContent.trim() === 'Approve'));
      return card ? card.textContent.trim().slice(0, 200) : null;
    })()`, 10000);
    const notYet = logged(ids.Ash, 'page').length;
    await shoot('web-computer-approval-1280x800');
    await click('article[aria-label="Approval"] button', 'Approve');
    const run = await finished(started.id);
    record('Ash\'s navigate shows an approval card "Open …/form", and Approve runs it',
      card.includes(summary) && notYet === 0 && run.status === 'completed' && run.result?.includes('Bruhs fixture form') && logged(ids.Ash, 'page').some((entry) => entry.path === '/form'),
      `card ${js(card)}; Ash pages before Approve ${notYet}; run ${run.status}; reply ${js(run.result?.slice(0, 120))}`);
  });

  // 6. evaluate is refused and never runs.
  await check('Pix\'s evaluate is refused with the not-for-bruhs text, and nothing ran', async () => {
    const before = fixtureLog.length;
    const run = await ask(ids.Pix, doLines({ action: 'evaluate', expression: "fetch('/log?kind=evaluated&value=1', { method: 'POST' })" }));
    await sleep(1000);
    const ran = fixtureLog.slice(before).filter((entry) => entry.kind === 'evaluated').length;
    const asked = steps('Pix').filter((entry) => entry.command?.action === 'evaluate');
    record('Pix\'s evaluate is refused with the not-for-bruhs text, and nothing ran',
      run.status === 'completed' && run.result?.includes(NOT_FOR_BRUHS) && ran === 0 && !asked.some((entry) => entry.phase === 'reply'),
      `reply ${js(run.result?.slice(0, 200))}; evaluated hits ${ran}; step phases ${asked.map((entry) => entry.phase).join(',')}`);
  });

  // 7. A page can't reach loopback, nor this computer's LAN address; a
  // navigate to loopback is refused.
  await check('the trap page reaches nothing on loopback; navigating to 127.0.0.1 or this computer\'s LAN address is refused', async () => {
    const before = loopbackConnections;
    const trap = await ask(ids.Pix, doLines({ action: 'navigate', url: `${FIXTURE}/trap` }));
    await until('the trap page reports', async () => logged(ids.Pix, 'trap').length > 0, 8000).catch(() => null);
    await sleep(1500);
    const trapOutcome = logged(ids.Pix, 'trap').at(-1)?.value;
    const loopback = await ask(ids.Pix, doLines({ action: 'navigate', url: `http://127.0.0.1:${countingPort}/` }));
    const lanRun = await ask(ids.Pix, doLines({ action: 'navigate', url: `http://${lanAddress}:${lanPort}/` }, { action: 'snapshot' }));
    record('the trap page reaches nothing on loopback; navigating to 127.0.0.1 or this computer\'s LAN address is refused',
      trap.status === 'completed' && trapOutcome === 'failed' && loopbackConnections - before === 0
        && loopback.result?.includes(LOCAL_DENIAL) && lanRun.result?.includes(LOCAL_DENIAL) && lanHits === 0,
      `trap fetch ${trapOutcome}; loopback connections ${loopbackConnections - before}; 127.0.0.1 reply ${js(loopback.result?.slice(0, 120))}; LAN ${lanAddress} reply ${js(lanRun.result?.slice(0, 160))}; LAN hits ${lanHits}`);
  });

  // (a) Downloads are denied: nothing is saved anywhere under HOME.
  await check('a download link saves nothing in Downloads or anywhere under HOME or the bruh\'s folder', async () => {
    const run = await ask(ids.Pix, doLines({ action: 'navigate', url: `${FIXTURE}/form` }, { action: 'click', selector: '#download' }));
    await until('the download was asked for', async () => logged(ids.Pix, 'download').length > 0, 8000).catch(() => null);
    await sleep(3000);
    const found = [...findNamed(home, 'bruhs-download-probe'), ...findNamed(home, '.crdownload'), ...findNamed(tree(ids.Pix), 'bruhs-download-probe'), ...findNamed(tree(ids.Pix), '.crdownload')];
    const downloads = fs.existsSync(path.join(home, 'Downloads')) ? fs.readdirSync(path.join(home, 'Downloads')) : [];
    record('a download link saves nothing in Downloads or anywhere under HOME or the bruh\'s folder',
      run.status === 'completed' && logged(ids.Pix, 'download').length > 0 && found.length === 0 && downloads.length === 0,
      `asked ${logged(ids.Pix, 'download').length}; found ${found.join(', ') || 'nothing'}; Downloads ${downloads.join(',') || 'empty or absent'}; click reply ${js(run.result?.slice(-160))}`);
  });

  // 9. A login's cookie survives Close and Open; Ash's browser has none.
  await check("Pix's cookie survives Close browser then Open browser; Ash's browser at the same URL has none", async () => {
    const login = await ask(ids.Pix, doLines({ action: 'navigate', url: `${FIXTURE}/login` }));
    await openBruh('Pix', 'Computer');
    await click(browserButton, 'Close browser');
    await waitStatus(lines.closed('Pix'));
    await click(browserButton, 'Open browser');
    await waitStatus(lines.has('Pix'));
    await serveFixture(ids.Pix);
    const run = await ask(ids.Pix, doLines({ action: 'navigate', url: `${FIXTURE}/form` }, { action: 'snapshot' }));
    const after = logged(ids.Pix, 'page').filter((entry) => entry.path === '/form').at(-1);
    // Ash at Ask first: its navigate waits for an approval, decided here.
    const before = (await runsOf(ids.Ash)).map((candidate) => candidate.id);
    await command({ action: 'chat', agent_id: ids.Ash, message: doLines({ action: 'navigate', url: `${FIXTURE}/form` }, { action: 'snapshot' }) });
    const ashRun = await newRun(ids.Ash, before);
    const item = await until("Ash's approval", async () => (await snapshot()).outbox.find((entry) => entry.run_id === ashRun.id && entry.approval && !entry.resolved_at), 10000);
    await command({ action: 'decide_approval', id: item.id, approve: true });
    const ashDone = await finished(ashRun.id);
    const ashPage = logged(ids.Ash, 'page').filter((entry) => entry.path === '/form').at(-1);
    const cookie = `bruh_cookie=${loginCookie}`;
    record("Pix's cookie survives Close browser then Open browser; Ash's browser at the same URL has none",
      login.status === 'completed' && after?.cookie.includes(cookie) && run.result?.includes(`Cookie: ${cookie}`)
        && ashDone.status === 'completed' && !!ashPage && ashPage.cookie === '' && ashDone.result?.includes('Cookie: none'),
      `Pix after reopen ${js(after?.cookie)}, page says ${js(run.result?.match(/Cookie: [^\\"]*/)?.[0])}; Ash request cookie ${js(ashPage?.cookie)}, page says ${js(ashDone.result?.match(/Cookie: [^\\"]*/)?.[0])}`);
  });

  // 10. The live view's socket: a foreign Origin is refused, and a page in
  // Pix's browser can't open it or read another bruh's state.
  await check("the screencast refuses Origin http://evil.example (403); Pix's own page can't open it or read Ash's state", async () => {
    const state = await computer(ids.Pix);
    const route = `/api/local-tools/browser/${state.session_id}/screencast`;
    const evil = await upgrade(route, 'http://evil.example');
    const own = await upgrade(route, base);
    const ashSession = (await computer(ids.Ash)).session_id;
    const run = await ask(ids.Pix, doLines({ action: 'navigate', url: `${FIXTURE}/socket?${new URLSearchParams({ port: String(appPort), session: state.session_id, agent: ids.Ash })}` }));
    await until('the socket page reports', async () => logged(ids.Pix, 'socket').length > 0 && logged(ids.Pix, 'state').length > 0, 10000).catch(() => null);
    const socket = logged(ids.Pix, 'socket').at(-1)?.value;
    const read = logged(ids.Pix, 'state').at(-1)?.value ?? '';
    record("the screencast refuses Origin http://evil.example (403); Pix's own page can't open it or read Ash's state",
      evil === 403 && own === 101 && run.status === 'completed' && socket === 'error' && read.startsWith('failed') && !read.includes(ashSession ?? 'no session'),
      `evil origin ${evil}; app origin ${own}; in-page socket ${socket}; in-page state read ${js(read.slice(0, 120))}`);
  });
  // (d) The state route is the owner's: a foreign origin can't read it.
  await check("GET …/agents/{id}/computer refuses a foreign origin, so no other page gets a session_id", async () => {
    const response = await fetch(`${base}/api/reactive/agents/${ids.Ash}/computer`, { headers: { origin: 'http://evil.example' } });
    const body = await response.text();
    record("GET …/agents/{id}/computer refuses a foreign origin, so no other page gets a session_id",
      response.status === 403 && !body.includes('session_id'), `status ${response.status}; body ${js(body.slice(0, 120))}`);
  });
  skip('a Full bruh\'s sandboxed shell cannot curl the server on loopback', 'needs a real Claude sandbox; Task 17b live check');

  // (e) A hidden tab keeps its live view past the 60 s ack deadline.
  await check('a hidden tab keeps its live view past the 60 s ack deadline', async () => {
    await ask(ids.Pix, doLines({ action: 'navigate', url: `${FIXTURE}/ticker` }));
    await openBruh('Pix', 'Computer');
    await waitFor('frames', `(() => { const img = document.querySelector('[data-computer-live]'); return !!img && img.naturalWidth > 0; })()`, 5000);
    await send('Emulation.setFocusEmulationEnabled', { enabled: false });
    const { windowId } = await send('Browser.getWindowForTarget');
    await send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'minimized' } });
    const hidden = await waitFor('the tab is hidden', `document.visibilityState === 'hidden'`, 3000).catch(() => false);
    const socketsBefore = [...liveSockets.values()].filter((socket) => !socket.closed).length;
    const start = Date.now();
    await sleep(65000);
    const lateFrames = framesSince(Date.now() - 10000);
    const allFrames = framesSince(start);
    const stalled = [...liveSockets.values()].flatMap((socket) => socket.texts).filter((text) => text.includes('stopped showing frames'));
    const closedSockets = [...liveSockets.values()].filter((socket) => socket.closed).length;
    await send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'normal' } });
    await send('Emulation.setFocusEmulationEnabled', { enabled: true });
    await waitFor('the tab is visible', `document.visibilityState === 'visible'`, 3000).catch(() => null);
    const error = await evaluate(`document.querySelector('[data-computer-error]')?.textContent.trim() ?? null`);
    record('a hidden tab keeps its live view past the 60 s ack deadline',
      !!hidden && socketsBefore >= 1 && allFrames > 100 && lateFrames > 0 && stalled.length === 0 && error === null,
      `hidden ${!!hidden}; frames while hidden ${allFrames}, in the last 10 s ${lateFrames}; stall messages ${stalled.length}; closed sockets ${closedSockets}; tab error ${js(error)}`);
  });

  // 8. Take control: the bruh's next step waits; the owner's click and
  // typing reach the page; Hand back returns the note and the run ends.
  await check("Take control holds the bruh's click; the owner's click and typing reach the page; Hand back returns the note", async () => {
    await ask(ids.Pix, doLines({ action: 'navigate', url: `${FIXTURE}/form` }));
    await openBruh('Pix', 'Computer');
    await waitFor('frames', `(() => { const img = document.querySelector('[data-computer-live]'); return !!img && img.naturalWidth > 0; })()`, 5000);
    await takeControl();
    const clicksBefore = logged(ids.Pix, 'click').length;
    const before = (await runsOf(ids.Pix)).map((run) => run.id);
    await command({ action: 'chat', agent_id: ids.Pix, message: doLines({ action: 'click', selector: '#press' }) });
    const run = await newRun(ids.Pix, before);
    await until("Pix asks for its click", async () => steps('Pix').some((entry) => entry.phase === 'ask' && entry.command.action === 'click' && entry.at >= run.created_at), 8000);
    await sleep(2000);
    const waiting = (await snapshot()).runs.find((candidate) => candidate.id === run.id)?.status;
    const heldClicks = logged(ids.Pix, 'click').length - clicksBefore;
    await shoot('web-computer-held-1280x800');
    await viewport(640, 480);
    await shoot('web-computer-held-640x480');
    await viewport(1280, 800);
    // The owner clicks Press, then types into the field.
    await clickView(250, 200);
    const ownerClick = await until('the owner\'s click lands', async () => logged(ids.Pix, 'click').length - clicksBefore === 1, 5000).catch(() => false);
    await clickView(250, 340);
    await sleep(300);
    for (const key of ['h', 'i']) await press(key);
    const typed = await until('the owner\'s typing lands', async () => logged(ids.Pix, 'typed').at(-1)?.value === 'hi', 5000).catch(() => false);
    // (f) ⌘V pastes the clipboard into the page as text.
    await send('Browser.grantPermissions', { origin: base, permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'] });
    await evaluate(`navigator.clipboard.writeText(' pasted')`);
    await evaluate(`document.querySelector('[data-computer-live]').focus()`);
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Meta', code: 'MetaLeft', windowsVirtualKeyCode: 91, modifiers: META });
    await press('v', META, { commands: ['paste'] });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Meta', code: 'MetaLeft', windowsVirtualKeyCode: 91, modifiers: 0 });
    const pasted = await until('the paste lands', async () => logged(ids.Pix, 'typed').at(-1)?.value === 'hi pasted', 5000).catch(() => false);
    record('⌘V in the live view pastes the clipboard into the page', !!pasted, `field ${js(logged(ids.Pix, 'typed').at(-1)?.value)}`);
    const stillWaiting = (await snapshot()).runs.find((candidate) => candidate.id === run.id)?.status;
    await handBack();
    const done = await finished(run.id);
    const note = 'The owner used your computer and handed it back. It is now on';
    record("Take control holds the bruh's click; the owner's click and typing reach the page; Hand back returns the note",
      waiting === 'running' && heldClicks === 0 && !!ownerClick && !!typed && stillWaiting === 'running'
        && done.status === 'completed' && done.result?.includes(note) && logged(ids.Pix, 'click').length - clicksBefore === 1,
      `while held ${waiting}, clicks ${heldClicks}; owner click ${!!ownerClick}; typed ${js(logged(ids.Pix, 'typed').at(-1)?.value)}; after hand back ${done.status}, reply ${js(done.result?.slice(0, 200))}; clicks ${logged(ids.Pix, 'click').length - clicksBefore}`);
  });

  // 14. Keyboard: F6 leaves the live view; ⌘. still stops.
  await check('with the live view focused, F6 moves focus to the toolbar and ⌘. still sends Stop all', async () => {
    await openBruh('Pix', 'Computer');
    await takeControl();
    await evaluate(`document.querySelector('[data-computer-live]').focus()`);
    const onView = (await active())?.live;
    const hint = await evaluate(`document.querySelector('[data-computer-hint]')?.textContent.trim()`);
    await press('F6');
    const afterF6 = await active();
    await evaluate(`document.querySelector('[data-computer-live]').focus()`);
    const since = Date.now();
    await press('.', META);
    const stopped = await until('Stop all is sent', async () => commandsSince(since, 'stop_all').length === 1, 3000).catch(() => false);
    const paused = await until('every bruh pauses', async () => (await snapshot()).bruhs.every((state) => state.status === 'paused'), 5000).catch(() => false);
    record('with the live view focused, F6 moves focus to the toolbar and ⌘. still sends Stop all',
      onView && hint === 'Keys go to the page. Press F6 to leave.' && afterF6?.address && !!stopped && !!paused,
      `on the view ${onView}; hint ${js(hint)}; after F6 ${js(afterF6)}; stop_all ${commandsSince(since, 'stop_all').length}; paused ${!!paused}`);
    await command({ action: 'resume_all' });
    await until('every bruh resumes', async () => (await snapshot()).bruhs.every((state) => state.status !== 'paused'), 5000);
  });

  // 15. 390×844 touch: 44px controls, the view fits, a tap clicks.
  await check('390×844 touch: toolbar controls are 44px, the live view fits, no sideways scroll, and a tap clicks in the page', async () => {
    await viewport(390, 844, true);
    await send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
    await openBruh('Pix');
    // Below sm the sidebar is a drawer over the page; close it with its scrim.
    if (await evaluate(`!!document.querySelector('div.fixed.inset-0.z-30')`)) {
      for (const kind of ['mousePressed', 'mouseReleased']) {
        await send('Input.dispatchMouseEvent', { type: kind, x: 385, y: 600, button: 'left', clickCount: 1 });
      }
      await waitFor('the drawer closes', `!document.querySelector('div.fixed.inset-0.z-30')`, 3000);
    }
    await showTab('Computer');
    await waitStatus(lines.held('Pix'));
    await waitFor('frames', `(() => { const img = document.querySelector('[data-computer-live]'); return !!img && img.naturalWidth > 0; })()`, 5000);
    await sleep(500);
    const layout = await evaluate(`(() => {
      const tab = document.querySelector('[data-bruh-computer]');
      const controls = [...tab.querySelectorAll('input, button')].filter((node) => node.getBoundingClientRect().height > 0).map((node) => [node.getAttribute('aria-label') ?? node.textContent.trim(), Math.round(node.getBoundingClientRect().height)]);
      const view = document.querySelector('[data-computer-live]').parentElement.getBoundingClientRect();
      return { controls, view: [Math.round(view.left), Math.round(view.right), Math.round(view.width)], width: innerWidth, sideways: document.scrollingElement.scrollWidth - innerWidth, coarse: matchMedia('(pointer: coarse)').matches };
    })()`);
    await shoot('web-computer-390x844-touch');
    const clicksBefore = logged(ids.Pix, 'click').length;
    const point = await viewPoint(250, 200);
    await send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: point.x, y: point.y }] });
    await sleep(80);
    await send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    const tapped = await until('the tap lands', async () => logged(ids.Pix, 'click').length - clicksBefore === 1, 5000).catch(() => false);
    await send('Emulation.setTouchEmulationEnabled', { enabled: false });
    await viewport(1280, 800);
    record('390×844 touch: toolbar controls are 44px, the live view fits, no sideways scroll, and a tap clicks in the page',
      layout.coarse && layout.controls.length >= 3 && layout.controls.every(([, height]) => height >= 44)
        && layout.view[0] >= 0 && layout.view[1] <= layout.width && layout.sideways <= 0 && !!tapped,
      `coarse ${layout.coarse}; controls ${layout.controls.map(([label, height]) => `${label}:${height}`).join(', ')}; view ${layout.view.join('/')} of ${layout.width}; sideways ${layout.sideways}; tap clicks ${logged(ids.Pix, 'click').length - clicksBefore}`);
  });

  // 11. Stop all while a step is held ends the run; the step never runs and
  // the browser stays open.
  await check("Stop all during a held step ends the run; the browser stays open and the step didn't run", async () => {
    await openBruh('Pix', 'Computer');
    await waitStatus(lines.held('Pix'));
    const clicksBefore = logged(ids.Pix, 'click').length;
    const before = (await runsOf(ids.Pix)).map((run) => run.id);
    await command({ action: 'chat', agent_id: ids.Pix, message: doLines({ action: 'click', selector: '#press' }) });
    const run = await newRun(ids.Pix, before);
    await until('Pix asks for its click', async () => steps('Pix').some((entry) => entry.phase === 'ask' && entry.at >= run.created_at), 8000);
    await click('[data-crew-stop-all]');
    const ended = await finished(run.id, 10000);
    await sleep(1500);
    const state = await computer(ids.Pix);
    const replied = steps('Pix').filter((entry) => entry.phase === 'reply' && entry.at >= run.created_at).length;
    record("Stop all during a held step ends the run; the browser stays open and the step didn't run",
      ended.status === 'cancelled' && !!state.session_id && state.control === 'owner' && logged(ids.Pix, 'click').length === clicksBefore && replied === 0,
      `run ${ended.status}; browser ${state.session_id ? 'open' : 'closed'}; control ${state.control}; clicks ${logged(ids.Pix, 'click').length - clicksBefore}; helper replies ${replied}`);
    await click('[data-crew-resume-all]');
    await until('every bruh resumes', async () => (await snapshot()).bruhs.every((candidate) => candidate.status !== 'paused'), 5000);
    await handBack();
  });

  // 16. Reduced motion: a working bruh's face stops on the Computer tab.
  await check('reduced motion leaves no animation frames on the Computer tab', async () => {
    hold('Pix');
    await command({ action: 'chat', agent_id: ids.Pix, message: 'Keep working.' });
    await until('Pix is working', async () => (await runsOf(ids.Pix)).some((run) => run.status === 'running'), 5000);
    await openBruh('Pix', 'Computer');
    await sleep(500);
    const framesOverASecond = async () => {
      await evaluate('window.__frames = 0');
      await sleep(1000);
      return evaluate('window.__frames');
    };
    const moving = await framesOverASecond();
    await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
    await sleep(300);
    const still = await framesOverASecond();
    await openBruh('Pix', 'Computer');
    await sleep(500);
    const stillAfterLoad = await framesOverASecond();
    record('reduced motion leaves no animation frames on the Computer tab', moving > 0 && still === 0 && stillAfterLoad === 0,
      `frames per second: moving ${moving}, reduced ${still}, reduced after reload ${stillAfterLoad}`);
    await send('Emulation.setEmulatedMedia', { features: [] });
    release('Pix');
    await settled(ids.Pix);
  });

  // (b) The cache stays in the bruh's tree.
  const cacheDir = path.join(tree(ids.Pix), 'browser-cache');
  const homeCaches = () => findNamed(path.join(home, 'Library', 'Caches'), '');
  await check("Pix's browser cache is in its tree, and nothing for it is under HOME/Library/Caches", async () => {
    const cached = fs.existsSync(cacheDir) ? findNamed(cacheDir, '').length : -1;
    const elsewhere = homeCaches();
    record("Pix's browser cache is in its tree, and nothing for it is under HOME/Library/Caches",
      cached > 0 && elsewhere.length === 0, `browser-cache entries ${cached}; HOME/Library/Caches ${elsewhere.slice(0, 5).join(', ') || 'empty or absent'}`);
  });

  // 12. Computer off closes the browser and shows the off line.
  await check('Computer off for Pix closes the browser, and the tab shows the off line', async () => {
    const pidFile = path.join(tree(ids.Pix), 'browser.pid');
    const pid = Number(fs.readFileSync(pidFile, 'utf8').trim());
    await openBruh('Pix', 'Settings');
    await click('[data-bruh-computer]');
    await click('form[aria-label="Bruh settings"] button', 'Save');
    await until('Computer is off for Pix', async () => (await snapshot()).bruhs.find((state) => state.agent_id === ids.Pix)?.computer === false, 5000);
    const exited = await until('Chrome exits', async () => {
      try {
        process.kill(pid, 0);
        return false;
      } catch {
        return true;
      }
    }, 10000).catch(() => false);
    await showTab('Computer');
    const off = await waitStatus(lines.off, 5000).then(() => true, async () => statusLine());
    const hidden = await evaluate(`document.querySelector('[data-computer-live]').parentElement.classList.contains('hidden')`);
    await shoot('web-computer-off-1280x800');
    record('Computer off for Pix closes the browser, and the tab shows the off line',
      !!exited && !fs.existsSync(pidFile) && off === true && hidden && fs.existsSync(path.join(tree(ids.Pix), 'browser')),
      `Chrome exited ${!!exited}; pid file ${fs.existsSync(pidFile)}; status ${js(off)}; live view hidden ${hidden}; profile kept ${fs.existsSync(path.join(tree(ids.Pix), 'browser'))}`);
  });

  // 13. Delete Pix asks first, then removes its whole tree.
  await check('Delete Pix asks first; after Delete its bruhs/<id> folder and browser cache are gone', async () => {
    await openApp('#/automations');
    // The agent cards sit under Advanced settings.
    await waitFor('the agent cards', `document.querySelectorAll('[data-agent-delete]').length >= 4`, 10000);
    await evaluate(`[...document.querySelectorAll('details')].filter((details) => details.querySelector('[data-agent-delete]')).forEach((details) => { details.open = true; })`);
    const mark = `(() => {
      const card = [...document.querySelectorAll('article')].find((node) => node.querySelector('[data-agent-delete]') && node.textContent.includes('Pix'));
      card?.querySelector('[data-agent-delete]')?.setAttribute('data-verify-delete', '');
      return !!card;
    })()`;
    if (!(await evaluate(mark))) throw new Error("Pix's card has no Delete agent");
    await click('[data-verify-delete]');
    const confirm = await waitFor('the question', `document.querySelector('[data-agent-delete-confirm]')?.textContent.trim() ?? null`, 3000);
    const onKeep = await waitFor('focus on Keep', `document.activeElement?.hasAttribute('data-agent-delete-keep')`, 3000).catch(() => false);
    const kept = (await snapshot()).agents.some((agent) => agent.id === ids.Pix) && fs.existsSync(tree(ids.Pix));
    await click('[data-agent-delete-yes]');
    const gone = await until('Pix is deleted', async () => !(await snapshot()).agents.some((agent) => agent.id === ids.Pix), 5000).catch(() => false);
    const removed = await until("Pix's folder goes", async () => !fs.existsSync(tree(ids.Pix)), 10000).catch(() => false);
    const expected = 'Delete Pix? Its chat, memory, browser, own vault entries and private work folder go with it. A folder you chose stays.';
    record('Delete Pix asks first; after Delete its bruhs/<id> folder and browser cache are gone',
      confirm.startsWith(expected) && !!onKeep && kept && !!gone && !!removed && !fs.existsSync(cacheDir) && homeCaches().length === 0,
      `asked ${js(confirm.slice(0, 120))}; focus on Keep ${!!onKeep}; kept until Delete ${kept}; deleted ${!!gone}; folder gone ${!!removed}; cache gone ${!fs.existsSync(cacheDir)}`);
  });

  record('no panics in the page', panics.length === 0, panics.join(' | '));
  record('screenshots saved', screenshots.length > 0 && screenshots.every((file) => fs.statSync(file).size > 0), screenshots.map((file) => path.basename(file)).join(', '));

  exitCode = results.every((result) => result.pass) ? 0 : 1;
  fs.writeFileSync(path.join(runDir, 'results.json'), JSON.stringify({ when: new Date().toISOString(), base, results, screenshots, fixtureLog }, null, 2));
  console.log(`Screenshots: ${shots}`);
  const failed = results.filter((result) => !result.pass).length;
  const skipped = results.filter((result) => result.skipped).length;
  console.log(`${results.length - failed - skipped}/${results.length - skipped} checks passed${skipped ? `, ${skipped} skipped` : ''}.`);
  console.log(exitCode === 0 ? 'All computer checks passed.' : `${failed} computer check(s) failed.`);
} catch (error) {
  console.error(`FAIL ${error.stack ?? error}`);
  fs.writeFileSync(path.join(runDir, 'results.json'), JSON.stringify({ when: new Date().toISOString(), results, error: String(error), fixtureLog }, null, 2));
} finally {
  await cleanup();
}
process.exit(exitCode);
