// Acceptance run for the vault on the web (docs/secure-credential-requests.md
// "The vault", docs/crew.md "Saved logins" and "Vault grants and keys";
// PARITY-19 in docs/web-desktop-parity-goal-2026-11-15.md): Settings, Vault
// from empty to four entries made by keyboard, a login's edit, the browser's
// storage, a bruh's grants, vault fills through a bruh's Computer (a Saved
// login with a one-time code, a granted login at Ask first, a 1Password
// login signed out then in), the refusals around them (wrong origin, not
// granted, screenshots and value selectors after a fill, keychain and
// 1Password shell lines, the vault's folder), a bruh's key use and a
// thread's, deleting a bruh, phone-size touch and reduced motion. Every
// value is a fresh random canary, and none may reach a tool result, the
// server log, an SSE frame, the page's DOM or storage, an app response or
// the reactive database. Any panic in the page fails the run. Every check
// prints ok or FAIL (or skip, with the reason); the exit status is 0 only
// when all pass.
//
//   cargo build -p bruhs-server
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43061 trunk build --dist "$TMPDIR/bruhs-vault-dist")
//   node scripts/verify-vault.mjs "$PWD" "$TMPDIR/bruhs-vault-dist" 43061
//
// The bundle must be built for the port the check uses. bruhs-server runs
// with BRUHS_SECRET_STORE=memory (a debug build keeps the keychain in
// memory, so the owner's keychain is never touched), BRUHS_OP_PATH naming a
// stand-in `op` this script writes (it answers three references, and a file
// `op-signed-out` beside it makes it answer as signed out), a disposable HOME
// and BRUHS_REACTIVE_DB, a capture URL nothing listens on and, first on its
// PATH, a stand-in `claude`, so no provider is called and no real data is
// touched.
//
// The stand-in `claude` is p3's (scripts/verify-computer.mjs), extended. It
// reads `--mcp-config=` and runs each line of its prompt in order:
//   do: {json}         a local_browser call: `can_use_tool` for
//                      mcp__bruhs_workbench__local_browser, then, once the
//                      server allows it, the bruhs_workbench helper's call.
//   do-bash: <line>    `can_use_tool` for Bash with that command.
//   do-read: <path>    `can_use_tool` for Read with that file_path.
//   do-secret: {json}  the bruhs_secrets helper's request_secret, as real
//                      Claude calls it (pre-allowed, so no can_use_tool).
// Every reply goes into its `result`, and into bin/steps.log.
//
// Fixtures. A bruh's browser refuses this computer, so the fixture pages
// live on names that never resolve, http://login.vault-fixture.test and
// http://other.vault-fixture.test, answered by this script over each bruh
// browser's own DevTools port with Fetch.requestPaused / fulfillRequest (as
// p3 does). `/in` has an email field, a password field, a "Show password"
// button and a form that posts to `/session`, then goes to `/code`, whose
// one-time code field is checked against the login's setup key. The page
// copies each field's value into its `value` attribute, as React does, so a
// selector probe could read it back. The fixture logs field lengths, never
// values.
//
// Headless Chrome (BRUHS_VERIFY_CHROME, default the macOS app) runs on
// BRUHS_VERIFY_CDP_PORT (default 9461) with its own profile. Screenshots,
// logs and results go to $TMPDIR/bruhs-vault-verify/, cleared on each run.
// Nothing written there holds a canary: results are sanitized first.
import { spawn, spawnSync } from 'node:child_process';
import { createHash, createHmac, randomBytes } from 'node:crypto';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const root = path.resolve(process.argv[2] ?? '');
const dist = path.resolve(process.argv[3] ?? '');
const appPort = Number(process.argv[4] ?? 43061);
const serverBin = path.join(root, 'target/debug/bruhs-server');
for (const [label, file] of [['bruhs-server', serverBin], ['web bundle', path.join(dist, 'index.html')]]) {
  if (!fs.existsSync(file)) {
    console.error(`${label} not found at ${file}; see the usage at the top of this script.`);
    process.exit(2);
  }
}
const cdpPort = Number(process.env.BRUHS_VERIFY_CDP_PORT ?? 9461);
const chromeBin = process.env.BRUHS_VERIFY_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const runDir = path.join(fs.realpathSync(tmpdir()), 'bruhs-vault-verify');
fs.rmSync(runDir, { recursive: true, force: true });
const home = path.join(runDir, 'home');
const bin = path.join(runDir, 'bin');
const opBin = path.join(runDir, 'op-bin');
const shots = path.join(runDir, 'shots');
// A thread's project must be under the server's HOME.
const project = path.join(runDir, 'home', 'project');
for (const dir of [home, bin, opBin, shots, project, path.join(runDir, 'chrome')]) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
const database = path.join(runDir, 'reactive.sqlite3');
const tree = (agentId) => path.join(path.dirname(database), 'bruhs', agentId);
// Where the vault's files would be (the in-memory store writes none):
// directories' data_local_dir for dev.bruhs.bruhs-ai under this HOME.
const credentials = path.join(home, 'Library', 'Application Support', 'dev.bruhs.bruhs-ai', 'credentials');
console.log(`Run directory: ${runDir}`);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const js = JSON.stringify;

// ── Canaries ─────────────────────────────────────────────────────────────────
const hex = (bytes = 12) => randomBytes(bytes).toString('hex');
const BASE32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
const base32 = (bytes) => {
  let bits = 0, value = 0, out = '';
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += BASE32[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += BASE32[(value << (5 - bits)) & 31];
  return out;
};
const totpSeed = randomBytes(20);
const totpAt = (seed, at) => {
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(at / 30000)));
  const mac = createHmac('sha1', seed).update(counter).digest();
  const offset = mac[mac.length - 1] & 15;
  return String((mac.readUInt32BE(offset) & 0x7fffffff) % 1000000).padStart(6, '0');
};
const tag = hex(3);
const canary = {
  pixUser: `pix-${hex(6)}@vault-fixture.test`,
  pixPass: `pw-${hex()}`,
  pixTotp: base32(totpSeed),
  globalUser: `ash-${hex(6)}@vault-fixture.test`,
  globalPass: `pw-${hex()}`,
  opUser: `op-${hex(6)}@vault-fixture.test`,
  opPass: `op-pw-${hex()}`,
  opOtp: String(100000 + (randomBytes(4).readUInt32BE() % 900000)),
  deployKey: `key-${hex(16)}`,
};
// Usernames are metadata (ruling F33a): the owner sees them on the Vault
// page. Every other canary is a value and must never come back anywhere.
const usernames = new Set(['pixUser', 'globalUser']);
const values = Object.entries(canary).filter(([name]) => !usernames.has(name));
const all = Object.entries(canary);
const names = {
  pix: `Pix shop ${tag}`,
  global: `Shared shop ${tag}`,
  key: 'DEPLOY_KEY',
  op: `Other shop ${tag}`,
};
const opRefs = {
  username: `op://Verify/Other${tag}/username`,
  password: `op://Verify/Other${tag}/password`,
  totp: `op://Verify/Other${tag}/otp?attribute=otp`,
};
const found = (text, set = values) => set.filter(([, value]) => String(text ?? '').includes(value)).map(([name]) => name);
const sanitize = (text) => all.reduce((out, [name, value]) => out.replaceAll(value, `[${name}]`), String(text ?? ''));
const deployHash = createHash('sha256').update(canary.deployKey).digest('hex');
const keyCommand = ['/bin/sh', '-c', `[ "$(printf %s "$DEPLOY_TOKEN" | /usr/bin/shasum -a 256 | /usr/bin/cut -c1-64)" = ${deployHash} ]`];
const keyRequest = { name: 'DEPLOY_KEY', reason: 'Check the deploy key reaches the command.', env_var: 'DEPLOY_TOKEN', command: keyCommand };

// ── The stand-in op ──────────────────────────────────────────────────────────
const signedOut = path.join(opBin, 'op-signed-out');
const opStandIn = path.join(opBin, 'op');
fs.writeFileSync(opStandIn, `#!${process.execPath}
const fs = require('fs');
const args = process.argv.slice(2);
fs.appendFileSync(${js(path.join(opBin, 'op.log'))}, JSON.stringify({ at: Date.now(), args: args.slice(0, 3), reference: args[3] }) + '\\n');
if (args[0] !== 'read' || args[1] !== '--no-newline' || args[2] !== '--' || args.length !== 4) {
  process.stderr.write('[ERROR] unknown command\\n');
  process.exit(1);
}
if (fs.existsSync(${js(signedOut)})) {
  process.stderr.write('[ERROR] 2026/11/19 12:00:00 account is not signed in\\n');
  process.exit(1);
}
const answers = ${js({ [opRefs.username]: canary.opUser, [opRefs.password]: canary.opPass, [opRefs.totp]: canary.opOtp })};
if (!(args[3] in answers)) {
  process.stderr.write('[ERROR] "' + args[3] + '" isn\\'t an item in any vault\\n');
  process.exit(1);
}
process.stdout.write(answers[args[3]]);
`, { mode: 0o755 });
fs.writeFileSync(signedOut, '');
const opCalls = () => {
  try {
    return fs.readFileSync(path.join(opBin, 'op.log'), 'utf8').split('\n').filter(Boolean).map((line) => JSON.parse(line));
  } catch {
    return [];
  }
};

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
const BROWSER = 'mcp__bruhs_workbench__local_browser';
const out = (event) => process.stdout.write(JSON.stringify(event) + '\\n');
const log = (file, entry) => fs.appendFileSync(path.join(__dirname, file), JSON.stringify({ at: Date.now(), ...entry }) + '\\n');
const streaming = args.includes('--input-format=stream-json');
const lines = readline.createInterface({ input: process.stdin })[Symbol.asyncIterator]();
const next = async () => (await lines.next()).value ?? '';
const text = (line) => {
  try {
    const message = JSON.parse(line);
    if (message.type !== 'user') return null;
    const content = message.message.content;
    return typeof content === 'string' ? content : content.map((part) => part.text ?? '').join('');
  } catch {
    return null;
  }
};
// An MCP helper the run's --mcp-config names, over stdio.
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
  let prompt = null;
  if (streaming) {
    while (prompt === null) prompt = text(await next());
  } else {
    prompt = args[args.indexOf('--') + 1] ?? '';
  }
  const bruh = (prompt.match(/Your name is ([^.\\n]+)\\./) ?? [])[1];
  const name = bruh ?? 'thread';
  const mcp = args.find((arg) => arg.startsWith('--mcp-config='))?.slice('--mcp-config='.length);
  const servers = mcp ? JSON.parse(mcp).mcpServers ?? {} : {};
  log('calls.log', { name, prompt, args: args.filter((arg) => !arg.startsWith('--mcp-config=')), servers: Object.keys(servers) });
  const session = 'verify-vault-' + name.toLowerCase() + '-' + Date.now();
  out({ type: 'system', subtype: 'init', session_id: session, model: 'claude-verify', tools: [], mcp_servers: Object.keys(servers).map((server) => ({ name: server, status: 'connected' })) });
  const steps = [...prompt.matchAll(/^(do|do-bash|do-read|do-secret): (.+)$/gm)].map((match) => ({ kind: match[1], arg: match[2] }));
  const replies = [];
  const started = new Map();
  const tools = async (server) => {
    if (!servers[server]) return null;
    if (!started.has(server)) {
      const tool = helper(servers[server]);
      await tool.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'verify-claude', version: '1' } });
      tool.notify('notifications/initialized');
      await tool.call('tools/list', {});
      started.set(server, tool);
    }
    return started.get(server);
  };
  const ask = async (tool, input, index) => {
    const requestId = name + '-' + Date.now() + '-' + index;
    out({ type: 'control_request', request_id: requestId, request: { subtype: 'can_use_tool', tool_name: tool, input, tool_use_id: 'toolu_' + requestId } });
    for (;;) {
      const line = await next();
      try {
        const message = JSON.parse(line);
        if (message.type === 'control_response' && message.response?.request_id === requestId) return message.response.response;
      } catch {}
    }
  };
  const said = (reply) => (reply.result?.content ?? []).filter((part) => part.type === 'text').map((part) => part.text).join('\\n');
  for (const [index, step] of steps.entries()) {
    let reply;
    if (step.kind === 'do-secret') {
      const secrets = await tools('bruhs_secrets');
      if (!secrets) {
        reply = 'No request_secret tool in this run.';
      } else {
        log('steps.log', { name, phase: 'ask', kind: step.kind, arg: step.arg });
        const answer = await secrets.call('tools/call', { name: 'request_secret', arguments: JSON.parse(step.arg) });
        reply = (answer.result?.isError ? 'Error: ' : '') + said(answer);
      }
    } else {
      const [tool, input] = step.kind === 'do'
        ? [BROWSER, { command: JSON.parse(step.arg) }]
        : step.kind === 'do-bash'
          ? ['Bash', { command: step.arg, description: 'Verify a shell line.' }]
          : ['Read', { file_path: step.arg }];
      if (step.kind === 'do' && !servers.bruhs_workbench) {
        reply = 'No browser tool in this run.';
      } else {
        log('steps.log', { name, phase: 'ask', kind: step.kind, command: input.command, arg: step.arg });
        const decision = await ask(tool, input, index);
        if (decision.behavior !== 'allow') {
          reply = 'Denied: ' + decision.message;
        } else if (step.kind !== 'do') {
          reply = 'Allowed: ' + tool;
        } else {
          const browser = await tools('bruhs_workbench');
          const answer = await browser.call('tools/call', { name: 'local_browser', arguments: decision.updatedInput ?? input });
          reply = (answer.result?.isError ? 'Error: ' : '') + said(answer);
        }
      }
    }
    log('steps.log', { name, phase: 'reply', kind: step.kind, arg: step.arg, reply: reply.slice(0, 4000) });
    replies.push(reply);
  }
  const answer = replies.length > 0 ? replies.join('\\n\\n') : 'Done.';
  if (!bruh) out({ type: 'assistant', message: { id: 'msg_' + session, role: 'assistant', model: 'claude-verify', content: [{ type: 'text', text: answer }] }, session_id: session });
  out({ type: 'result', subtype: 'success', is_error: false, result: answer, session_id: session, duration_ms: 1 });
  process.exit(0);
})();
`, { mode: 0o755 });
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
const sockets = new Set();
const aborts = new Set();
// Every process below `pid`, so helpers the server and stand-ins started
// can be stopped once their parents are gone.
const descendants = (pids) => {
  const table = spawnSync('/bin/ps', ['-axo', 'pid=,ppid='], { encoding: 'utf8' }).stdout.trim().split('\n').map((line) => line.trim().split(/\s+/).map(Number));
  const found = new Set();
  let frontier = pids.filter(Boolean);
  while (frontier.length > 0) {
    const nextLevel = table.filter(([, ppid]) => frontier.includes(ppid)).map(([pid]) => pid).filter((pid) => !found.has(pid));
    for (const pid of nextLevel) found.add(pid);
    frontier = nextLevel;
  }
  return [...found];
};
async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once('exit', resolve));
  child.kill('SIGTERM');
  if (await Promise.race([exited.then(() => true), sleep(8000).then(() => false)]) === false) child.kill('SIGKILL');
  children.delete(child);
}
let cleaned = false;
async function cleanup() {
  if (cleaned) return;
  cleaned = true;
  for (const abort of aborts) abort.abort();
  for (const socket of sockets) try { socket.close(); } catch {}
  const below = descendants([...children].map((child) => child.pid));
  for (const child of [...children].reverse()) await stopChild(child);
  for (const pid of below) try { process.kill(pid, 'SIGKILL'); } catch {}
  spawnSync('/usr/bin/pkill', ['-f', standIn]);
  // A bruh browser the server didn't get to close: its profile is in this run.
  spawnSync('/usr/bin/pkill', ['-f', `--user-data-dir=${path.dirname(database)}/bruhs/`]);
}
process.on('SIGINT', () => cleanup().then(() => process.exit(130)));

const results = [];
const record = (name, pass, detail = '') => {
  const shown = sanitize(detail);
  results.push({ name, pass: !!pass, detail: shown.slice(0, 1200) });
  console.log(`${pass ? 'ok  ' : 'FAIL'} ${name}${shown ? ` (${shown.slice(0, 700)})` : ''}`);
  return !!pass;
};
// BRUHS_VERIFY_ONLY, a regular expression, runs only the checks whose
// names match (later checks may need earlier ones). Such a partial run
// never passes.
const only = process.env.BRUHS_VERIFY_ONLY ? new RegExp(process.env.BRUHS_VERIFY_ONLY) : null;
const check = async (name, body) => {
  if (only && !only.test(name)) return;
  try {
    await body();
  } catch (error) {
    record(name, false, error?.message ?? String(error));
  }
};

// ── The fixture site, answered over each bruh browser's DevTools port ───────
const LOGIN = 'http://login.vault-fixture.test';
const OTHER = 'http://other.vault-fixture.test';
const fixtureLog = [];
const logged = (agent, kind, host) => fixtureLog.filter((entry) => entry.agent === agent && entry.kind === kind && (!host || entry.host === host));
const signInPage = () => `<!doctype html><meta charset=utf-8><title>Vault fixture sign in</title>
<style>body{margin:0;font:16px sans-serif;background:#fff;padding:24px}label{display:block;margin:12px 0}input{width:280px;height:32px}</style>
<h1>Sign in to the vault fixture</h1>
<form id=sign-in method=post action=/session>
<label>Email <input id=email name=email type=email autocomplete=username placeholder="you@example.test"></label>
<label>Password <input id=pw name=password type=password autocomplete=current-password placeholder="Password"></label>
<button type=button id=show>Show password</button>
<button type=submit id=go>Sign in</button>
</form>
<script>
const log = (kind, value) => fetch('/log?' + new URLSearchParams({ kind, value }), { method: 'POST' }).catch(() => {});
// As React does for a controlled input: the value attribute follows the field.
for (const input of document.querySelectorAll('input')) input.addEventListener('input', () => { input.setAttribute('value', input.value); log('input', input.id + ':' + input.value.length); });
document.getElementById('show').addEventListener('click', () => { const pw = document.getElementById('pw'); pw.type = pw.type === 'password' ? 'text' : 'password'; log('show', pw.type); });
document.getElementById('sign-in').addEventListener('submit', () => log('submit', 'sign-in'));
</script>`;
const codePage = () => `<!doctype html><meta charset=utf-8><title>Vault fixture code</title>
<style>body{margin:0;font:16px sans-serif;background:#fff;padding:24px}input{width:200px;height:32px}</style>
<h1>Enter your one-time code</h1>
<form id=verify method=post action=/verify><label>Code <input id=code name=code inputmode=numeric autocomplete=one-time-code></label><button type=submit>Verify</button></form>
<script>
const log = (kind, value) => fetch('/log?' + new URLSearchParams({ kind, value }), { method: 'POST' }).catch(() => {});
document.getElementById('code').addEventListener('input', (event) => log('input', 'code:' + event.target.value.length));
</script>`;
const page = (title, body) => `<!doctype html><meta charset=utf-8><title>${title}</title><p>${body}</p>`;
// Which login's canaries a sign-in carried, by name; never the values.
const whose = (email, password) => {
  for (const [who, user, pass] of [['pix', canary.pixUser, canary.pixPass], ['global', canary.globalUser, canary.globalPass], ['op', canary.opUser, canary.opPass]]) {
    if (email === user || password === pass) return { who, user: email === user, pass: password === pass };
  }
  return { who: 'none', user: false, pass: false, lengths: [email?.length ?? 0, password?.length ?? 0] };
};
const bodyOf = (request) => {
  if (typeof request.postData === 'string') return request.postData;
  return (request.postDataEntries ?? []).map((entry) => Buffer.from(entry.bytes ?? '', 'base64').toString('utf8')).join('');
};
async function fulfill(agent, cdp, sessionId, params) {
  const url = new URL(params.request.url);
  const entry = { agent, at: Date.now(), host: url.host, path: url.pathname, kind: 'page' };
  let status = 200;
  let headers = [{ name: 'content-type', value: 'text/html; charset=utf-8' }, { name: 'cache-control', value: 'no-store' }];
  let body = '';
  if (url.pathname === '/log') {
    entry.kind = url.searchParams.get('kind');
    entry.value = url.searchParams.get('value');
    status = 204;
  } else if (url.pathname === '/in') {
    body = signInPage();
  } else if (url.pathname === '/session' && params.request.method === 'POST') {
    const form = new URLSearchParams(bodyOf(params.request));
    Object.assign(entry, { kind: 'session', ...whose(form.get('email'), form.get('password')) });
    status = 303;
    headers = [{ name: 'location', value: '/code' }, { name: 'cache-control', value: 'no-store' }];
  } else if (url.pathname === '/code') {
    body = codePage();
  } else if (url.pathname === '/verify' && params.request.method === 'POST') {
    const code = new URLSearchParams(bodyOf(params.request)).get('code') ?? '';
    const now = Date.now();
    const ok = [-30000, 0, 30000].some((skew) => totpAt(totpSeed, now + skew) === code);
    Object.assign(entry, { kind: 'verify', ok, length: code.length });
    body = page('Vault fixture verified', ok ? 'Welcome back.' : 'That code is wrong.');
  } else {
    status = 404;
    body = 'Not here.';
  }
  fixtureLog.push(entry);
  await cdp.send('Fetch.fulfillRequest', { requestId: params.requestId, responseCode: status, responseHeaders: headers, body: Buffer.from(body).toString('base64') }, sessionId).catch(() => {});
}
const bruhBrowsers = new Map();
async function serveFixture(agent, timeout = 15000) {
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
        await cdp.send('Fetch.enable', { patterns: [LOGIN, OTHER].map((origin) => ({ urlPattern: `${origin}/*`, requestStage: 'Request' })) }, sessionId);
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

// Every readable file under `dir`, as bytes, for a leak scan.
const filesUnder = (dir, skip = () => false) => {
  const out = [];
  const walk = (folder) => {
    let entries = [];
    try {
      entries = fs.readdirSync(folder, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(folder, entry.name);
      if (skip(full)) continue;
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) out.push(full);
    }
  };
  walk(dir);
  return out;
};
const fileLeaks = (files, set = values) => files.flatMap((file) => {
  let bytes;
  try {
    bytes = fs.readFileSync(file);
  } catch {
    return [];
  }
  return set.filter(([, value]) => bytes.includes(Buffer.from(value))).map(([name]) => `${path.relative(runDir, file)}:${name}`);
});
const databaseFiles = () => [database, `${database}-wal`, `${database}-shm`].filter((file) => fs.existsSync(file));
const serverLog = () => fs.readFileSync(path.join(runDir, 'server.log'), 'utf8');

let exitCode = 1;
try {
  const logFile = fs.openSync(path.join(runDir, 'server.log'), 'a');
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
      BRUHS_SECRET_STORE: 'memory',
      BRUHS_OP_PATH: opStandIn,
      RUST_LOG: 'info',
    },
    stdio: ['ignore', logFile, logFile],
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
    const text = await response.text();
    if (!response.ok) throw new Error(`${method} ${route}: ${response.status} ${text}`);
    return text ? JSON.parse(text) : null;
  };
  const command = (body) => api('POST', '/api/reactive/commands', body);
  const snapshot = () => api('GET', '/api/reactive');
  const vault = () => api('GET', '/api/vault');
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
  const newRun = (agentId, before, timeout = 8000) =>
    until(`a new run for ${agentId}`, async () => (await runsOf(agentId)).find((run) => !before.includes(run.id)), timeout);
  const steps = (name) => readLog('steps.log').filter((entry) => entry.name === name);
  const doLines = (...commands) => commands.map((step) => (typeof step === 'string' ? step : `do: ${js(step)}`)).join('\n');
  // A chat as `message_bruh` sends it. Each open approval of the run is
  // decided by `decide(item)` (true approves, false denies, undefined leaves
  // it for the caller); every one seen is returned.
  const ask = async (agentId, message, { decide = () => true, timeout = 60000 } = {}) => {
    const before = (await runsOf(agentId)).map((run) => run.id);
    await command({ action: 'chat', agent_id: agentId, message });
    const started = await newRun(agentId, before);
    const approvals = [];
    const seen = new Set();
    const deadline = Date.now() + timeout;
    for (;;) {
      const state = await snapshot();
      const run = state.runs.find((candidate) => candidate.id === started.id);
      for (const item of state.outbox.filter((entry) => entry.run_id === started.id && entry.approval && !entry.resolved_at && !seen.has(entry.id))) {
        seen.add(item.id);
        approvals.push(item);
        const verdict = decide(item);
        if (verdict !== undefined) await command({ action: 'decide_approval', id: item.id, approve: verdict });
      }
      if (run && !live(run)) return { run, approvals };
      if (Date.now() > deadline) throw new Error(`Timed out: run ${started.id} for ${agentId}`);
      await sleep(150);
    }
  };
  const replies = (name, since) => steps(name).filter((entry) => entry.phase === 'reply' && entry.at >= since).map((entry) => entry.reply);

  // The live stream, read from its first frame to the end of the run.
  const streamFrames = [];
  {
    const abort = new AbortController();
    aborts.add(abort);
    fetch(`${base}/api/reactive/stream`, { headers: { origin: base }, signal: abort.signal }).then(async (response) => {
      const decoder = new TextDecoder();
      for await (const chunk of response.body) streamFrames.push(decoder.decode(chunk, { stream: true }));
    }).catch(() => {});
  }

  // ── Seed: Pix (Full), Ash and Rex (Ask first), every Computer on ──────────
  const ids = { Pix: `vault-pix-${tag}`, Ash: `vault-ash-${tag}`, Rex: `vault-rex-${tag}` };
  const seed = [
    ['Pix', 'full', ['left', 'blueberry', 'bruh']],
    ['Ash', 'ask', ['front', 'pandan', 'bruh']],
    ['Rex', 'ask', ['right', 'mangosteen', 'longan']],
  ];
  for (const [name, level, [look, face, mouth]] of seed) {
    await command({
      action: 'create_bruh',
      agent: { id: ids[name], name, instructions: `${name} signs in to sites for the owner.`, model: 'claude-sonnet-4-6', project_path: '', enabled: true },
      look: { look, face, mouth },
      access: { level, send_as_me: false, can_deliver: false },
    });
    await command({ action: 'set_computer', agent_id: ids[name], on: true });
  }

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
  const tab = await connect(targets.find((t) => t.type === 'page').webSocketDebuggerUrl);
  sockets.add(tab.socket);
  const { send, listeners } = tab;
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
      const covered = !(hit && (hit === element || element.contains(hit) || hit.contains(element)));
      return { x, y, covered, by: covered && hit ? hit.tagName + '.' + [...hit.classList].join('.') : null, disabled: !!element.disabled };
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
  const keys = { Tab: ['Tab', 9], Enter: ['Enter', 13], Escape: ['Escape', 27] };
  const press = async (key, modifiers = 0) => {
    const [code, keyCode] = keys[key] ?? [`Key${key.toUpperCase()}`, key.toUpperCase().charCodeAt(0)];
    const typed = key === 'Enter' ? '\r' : key.length === 1 ? key : null;
    const text = modifiers === 0 && typed ? { text: typed, unmodifiedText: typed } : {};
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode: keyCode, modifiers, ...text });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: keyCode, modifiers });
  };
  const type = (text) => send('Input.insertText', { text });
  const viewport = (width, height, mobile = false) =>
    send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile });
  const screenshots = [];
  const shoot = async (name) => {
    await sleep(300);
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    const file = path.join(shots, `${name}.png`);
    fs.writeFileSync(file, Buffer.from(data, 'base64'));
    screenshots.push(file);
    return file;
  };
  // What the focused element is: its tag, its label's text and whether it
  // is read only.
  const focused = () => evaluate(`(() => {
    const element = document.activeElement;
    if (!element) return null;
    const label = element.labels?.[0]?.textContent.trim() ?? element.getAttribute('aria-label') ?? element.textContent.trim().slice(0, 40);
    return { tag: element.tagName, label, readOnly: !!element.readOnly, type: element.type ?? null, value: element.tagName === 'SELECT' ? element.value : null, inForm: !!element.closest('[data-vault-form]') };
  })()`);
  // Tab until the field labelled `label` has focus, by keyboard only.
  const tabTo = async (label, limit = 20) => {
    for (let count = 0; count < limit; count++) {
      const at = await focused();
      if (at?.label === label && at.inForm) return at;
      await press('Tab');
    }
    throw new Error(`Tab never reached "${label}" (at ${js(await focused())})`);
  };
  // Choose an option of the focused select by typing, as a keyboard user
  // does; the same letter again moves to the next option starting with it.
  const choose = async (letters, value) => {
    for (const letter of letters) {
      await send('Input.dispatchKeyEvent', { type: 'keyDown', key: letter, code: /\d/.test(letter) ? `Digit${letter}` : `Key${letter.toUpperCase()}`, windowsVirtualKeyCode: letter.toUpperCase().charCodeAt(0), text: letter, unmodifiedText: letter });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key: letter, code: /\d/.test(letter) ? `Digit${letter}` : `Key${letter.toUpperCase()}`, windowsVirtualKeyCode: letter.toUpperCase().charCodeAt(0) });
      await sleep(80);
    }
    const at = await focused();
    if (at?.value !== value) throw new Error(`typing ${js(letters)} chose ${js(at?.value)}, not ${js(value)}`);
  };

  // Any Rust panic in the page fails the run. Every app response body and
  // every event-stream frame the page sees is kept for the leak scans.
  const panics = [];
  const sent = [];
  const responses = new Map();
  const bodies = [];
  const pageFrames = [];
  listeners.add((message) => {
    if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
      const text = message.params.args.map((arg) => arg.value ?? arg.description ?? '').join(' ');
      if (text.includes('panicked')) panics.push(text.slice(0, 300));
    }
    if (message.method === 'Runtime.exceptionThrown') {
      const text = message.params.exceptionDetails?.exception?.description ?? message.params.exceptionDetails?.text ?? '';
      if (text.includes('panicked') || text.includes('unreachable')) panics.push(text.slice(0, 300));
    }
    if (message.method === 'Network.requestWillBeSent') {
      const { request } = message.params;
      if (request.method === 'POST' && request.url.endsWith('/api/reactive/commands')) {
        try {
          sent.push({ at: Date.now(), ...JSON.parse(request.postData ?? 'null') });
        } catch {}
      }
    }
    if (message.method === 'Network.responseReceived' && message.params.response.url.startsWith(base)) {
      responses.set(message.params.requestId, message.params.response.url);
    }
    if (message.method === 'Network.loadingFinished' && responses.has(message.params.requestId)) {
      const url = responses.get(message.params.requestId);
      responses.delete(message.params.requestId);
      send('Network.getResponseBody', { requestId: message.params.requestId })
        .then(({ body, base64Encoded }) => bodies.push({ url, body: base64Encoded ? Buffer.from(body, 'base64').toString('utf8') : body }))
        .catch(() => {});
    }
    if (message.method === 'Network.eventSourceMessageReceived') pageFrames.push(message.params.data);
  });
  const commandsSince = (since, action) => sent.filter((entry) => entry.at >= since && entry.action === action);
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Network.enable', { maxTotalBufferSize: 64 * 1024 * 1024, maxResourceBufferSize: 8 * 1024 * 1024 });
  await send('Emulation.setFocusEmulationEnabled', { enabled: true });
  await send('Page.addScriptToEvaluateOnNewDocument', {
    source: `window.__frames = 0; const request = window.requestAnimationFrame.bind(window);
      window.requestAnimationFrame = (callback) => { window.__frames += 1; return request(callback); };`,
  });
  await viewport(1280, 800);
  // A full load each time; `selector` is what shows the view has drawn.
  const openApp = async (hash = '', selector = '[data-crew-rail]') => {
    await send('Page.navigate', { url: 'about:blank' });
    await waitFor('a blank page', `location.href === 'about:blank'`);
    await send('Page.navigate', { url: `${base}/${hash}` });
    await waitFor(`the app has loaded ${hash}`, `document.readyState === 'complete' && !!document.querySelector(${js(selector)})`, 20000);
  };
  const ready = `!!document.querySelector('#vault [data-vault-new]') && !document.querySelector('#vault-status [role="status"]')?.textContent.includes('Loading')`;
  const openVault = async () => {
    await openApp('#/settings', '#vault');
    await waitFor('the Vault section', ready, 15000);
    await evaluate(`document.querySelector('#vault').scrollIntoView({ block: 'start' })`);
  };
  const openBruh = async (name, tabName) => {
    await openApp(`#/bruhs/${ids[name]}`);
    await waitFor(`${name}'s page`, `document.querySelector('#bruh-title')?.textContent.trim() === ${js(name)}`);
    if (tabName) {
      await click('[role="tab"]', tabName);
      if (tabName === 'Settings') await waitFor('the Settings form', `!!document.querySelector('form[aria-label="Bruh settings"] [data-bruh-vault]')`);
      if (tabName === 'Chat') await waitFor('the chat', `!!document.querySelector('textarea[aria-label="Message"]')`);
    }
  };
  // Every row on the page as its lines of text.
  const rows = () => evaluate(`[...document.querySelectorAll('#vault [data-vault-row]')].map((row) => ({ id: row.getAttribute('data-vault-row'), label: row.getAttribute('aria-label'), lines: [...row.querySelectorAll('h3, p')].map((node) => node.textContent.trim()), spans: [...row.querySelectorAll('h3 + p span')].map((node) => node.textContent.trim()) }))`);
  // What the page shows and holds: its text, its markup with every
  // attribute, and every field's live value.
  const pageText = () => evaluate(`document.documentElement.outerHTML + '\\n' + document.body.innerText + '\\n' + [...document.querySelectorAll('input, textarea, select')].map((field) => field.value).join('\\n')`);
  const COPY = {
    empty: 'The vault is empty. Choose New entry to add a login, a key or a 1Password reference.',
    saved: 'Stored in your OS keychain. Value hidden.',
    fromOp: 'Read from 1Password when used.',
    fixed: "Delete it and create it again to change its name or where it's used.",
    opNeedsYou: '1Password needs you. Sign in to the 1Password app, then check again.',
    ready: 'Ready.',
    notGranted: "That vault login isn't granted to you. The owner can grant it in your Settings.",
    wrongOrigin: 'That login fills only on its own site. Open its sign-in page first; logins lists it.',
    screenshot: 'Screenshots are off on this page after a vault fill. Take a snapshot, or go on to the next page.',
    selector: 'After a vault fill, pick elements on this page by id, class, tag, position or an exact attribute such as [type=password] or [name=user], never value or value states such as :valid. Take a snapshot to find them.',
    opOwner: '1Password needs the owner to sign in on this computer. Stop here and say so.',
    command: "Bruhs can't read the keychain or 1Password from a shell. Fill a vault login in your browser instead.",
    file: "That path holds the owner's saved logins. Leave it alone.",
    grantsHint: 'Logins it may fill on their own sites without asking. A key still asks each time it\'s used.',
  };

  // The three bruh browsers, opened by the owner so the fixture is served
  // from their first page on.
  for (const name of ['Pix', 'Ash', 'Rex']) {
    await api('POST', `/api/reactive/agents/${ids[name]}/computer`, { action: 'open' });
    await serveFixture(ids[name]);
  }

  // 1. Settings, Vault starts empty and finds the stand-in op.
  await check('Settings, Vault: the empty line shows and the 1Password CLI line is absent', async () => {
    await openVault();
    const state = await evaluate(`({ status: document.querySelector('#vault-status')?.textContent.trim(), missing: !!document.querySelector('[data-vault-op-missing]'), rows: document.querySelectorAll('[data-vault-row]').length })`);
    const listing = await vault();
    await shoot('web-vault-empty-1280x800');
    record('Settings, Vault: the empty line shows and the 1Password CLI line is absent',
      state.status === COPY.empty && !state.missing && state.rows === 0 && listing.entries.length === 0 && listing.one_password === true,
      `status ${js(state.status)}; 1Password line ${state.missing}; rows ${state.rows}; listing ${listing.entries.length} entries, one_password ${listing.one_password}`);
  });

  // 2. Four entries by keyboard.
  const entryIds = {};
  await check('New entry by keyboard: a Pix login, a shared login, DEPLOY_KEY and a 1Password login; rows read as pinned; no value in any response, DOM text or attribute', async () => {
    await openVault();
    const bodiesBefore = bodies.length;
    const create = async (fill) => {
      await evaluate(`document.querySelector('[data-vault-new]').focus()`);
      await press('Enter');
      await waitFor('the form takes focus', `!!document.activeElement?.closest('[data-vault-form]')`, 3000);
      await fill();
      const count = await evaluate(`document.querySelectorAll('[data-vault-row]').length`);
      await press('Enter');
      await waitFor('the entry is saved', `!document.querySelector('[data-vault-new]').getAttribute('aria-expanded').includes('true') && document.querySelectorAll('[data-vault-row]').length === ${count + 1}`, 8000)
        .catch(async () => { throw new Error(`not saved: ${sanitize(await evaluate(`document.querySelector('[data-vault-form] [role="alert"]')?.textContent ?? 'no error shown'`))}`); });
    };
    // A login only Pix uses, with a one-time code.
    await create(async () => {
      if ((await focused()).label !== 'Kind') throw new Error(`the form opened on ${js(await focused())}`);
      await tabTo('Name');
      await type(names.pix);
      await tabTo('Used by');
      await choose('oo', 'bruh');
      await tabTo('Bruh');
      await choose('p', ids.Pix);
      await tabTo('Sign-in page');
      await type(`${LOGIN}/in`);
      await tabTo('Username');
      await type(canary.pixUser);
      await tabTo('Password');
      await type(canary.pixPass);
      await tabTo('One-time code setup key');
      await type(canary.pixTotp);
      await shoot('web-vault-new-login-1280x800');
    });
    // A login for all projects on the same site.
    await create(async () => {
      await tabTo('Name');
      await type(names.global);
      await tabTo('Sign-in page');
      await type(`${LOGIN}/in`);
      await tabTo('Username');
      await type(canary.globalUser);
      await tabTo('Password');
      await type(canary.globalPass);
    });
    // A key for all projects.
    await create(async () => {
      await choose('k', 'value');
      await tabTo('Name');
      await type(names.key);
      await tabTo('Value');
      await type(canary.deployKey);
    });
    // A 1Password login on another site.
    await create(async () => {
      await tabTo('Name');
      await type(names.op);
      await tabTo('Sign-in page');
      await type(`${OTHER}/in`);
      await tabTo('Stored in');
      await choose('1', 'one_password');
      await tabTo('Username reference');
      await type(opRefs.username);
      await tabTo('Password reference');
      await type(opRefs.password);
      await tabTo('One-time code reference');
      await type(opRefs.totp);
      await shoot('web-vault-new-1password-1280x800');
    });
    const listing = await vault();
    for (const [key, name] of Object.entries(names)) entryIds[key] = listing.entries.find((entry) => entry.name === name)?.id;
    await sleep(500);
    const shown = await rows();
    await shoot('web-vault-list-1280x800');
    await viewport(640, 480);
    await shoot('web-vault-list-640x480');
    await viewport(1280, 800);
    const byName = Object.fromEntries(shown.map((row) => [row.lines[0], row]));
    const expect = {
      [names.pix]: ['Login', "Pix's own", `${LOGIN} as ${canary.pixUser}, with a one-time code`, COPY.saved],
      [names.global]: ['Login', 'All projects', `${LOGIN} as ${canary.globalUser}`, COPY.saved],
      [names.key]: ['Key', 'All projects', COPY.saved],
      [names.op]: ['Login', 'All projects', `${OTHER} from 1Password`, COPY.fromOp],
    };
    const misread = Object.entries(expect).filter(([name, lines]) => {
      const row = byName[name];
      if (!row) return true;
      const [kind, scope, ...rest] = lines;
      return row.spans[0] !== kind || row.spans[1] !== scope || js(row.lines.slice(2)) !== js(rest) || row.label !== `Vault entry: ${name}`;
    }).map(([name]) => `${name}: ${js(byName[name]?.lines)}`);
    const text = await pageText();
    const inPage = found(text);
    await sleep(500);
    const inResponses = bodies.slice(bodiesBefore).flatMap((entry) => found(entry.body).map((name) => `${new URL(entry.url).pathname}:${name}`));
    const inListing = found(js(listing));
    const scopes = Object.fromEntries(listing.entries.map((entry) => [entry.name, entry.scope.type === 'bruh' ? `bruh:${entry.scope.agent_id}` : entry.scope.type]));
    record('New entry by keyboard: a Pix login, a shared login, DEPLOY_KEY and a 1Password login; rows read as pinned; no value in any response, DOM text or attribute',
      Object.values(entryIds).every(Boolean) && misread.length === 0 && inPage.length === 0 && inResponses.length === 0 && inListing.length === 0
        && scopes[names.pix] === `bruh:${ids.Pix}` && scopes[names.global] === 'global' && scopes[names.key] === 'global' && scopes[names.op] === 'global',
      `ids ${js(entryIds)}; scopes ${js(scopes)}; misread ${misread.join(' | ') || 'none'}; values in the page ${inPage.join(',') || 'none'}, in responses ${inResponses.join(',') || 'none'} of ${bodies.length - bodiesBefore}, in the listing ${inListing.join(',') || 'none'}`);
  });
  // The grants the later checks need (ruling F33b): Pix gets DEPLOY_KEY and
  // the 1Password login here; Ash gets the shared login through its page.
  if (entryIds.key && entryIds.op) await command({ action: 'set_vault', agent_id: ids.Pix, entries: [entryIds.key, entryIds.op].sort() });

  // 2b. A login's edit: the sign-in page is read only, and focus starts on
  // the first field the owner can change.
  await check("a login's Edit shows the sign-in page read only with the fixed hint, focuses the first field it can change, and Escape returns to Edit", async () => {
    await openVault();
    const row = `[data-vault-row="${entryIds.pix}"]`;
    await click(`${row} [data-vault-edit]`);
    await waitFor('the edit form', `!!document.querySelector('${row} [data-vault-form]')`, 3000);
    await sleep(300);
    const form = await evaluate(`(() => {
      const form = document.querySelector('${row} [data-vault-form]');
      const field = (label) => [...form.querySelectorAll('input, select')].find((node) => node.labels?.[0]?.textContent.trim() === label);
      const signIn = field('Sign-in page');
      const first = form.querySelector('input:not([readonly]), select');
      return { signIn: signIn && { readOnly: signIn.readOnly, value: signIn.value }, hint: [...form.querySelectorAll('p')].map((node) => node.textContent.trim()).includes(${js(COPY.fixed)}), fixed: [...form.querySelectorAll('input[readonly]')].map((node) => node.labels?.[0]?.textContent.trim()), first: first?.labels?.[0]?.textContent.trim(), focusIsFirst: document.activeElement === first, password: field('Password')?.value, totp: field('One-time code setup key')?.value };
    })()`);
    await shoot('web-vault-edit-login-1280x800');
    await press('Escape');
    const back = await waitFor('focus back on Edit', `document.activeElement?.matches('${row} [data-vault-edit]') && !document.querySelector('${row} [data-vault-form]')`, 3000).catch(() => false);
    record("a login's Edit shows the sign-in page read only with the fixed hint, focuses the first field it can change, and Escape returns to Edit",
      form.signIn?.readOnly === true && form.signIn.value === `${LOGIN}/in` && form.hint && js(form.fixed) === js(['Kind', 'Name', 'Used by', 'Sign-in page'])
        && form.focusIsFirst && form.first !== 'Sign-in page' && form.password === '' && form.totp === '' && !!back,
      `sign-in ${js(form.signIn)}; hint ${form.hint}; read only ${js(form.fixed)}; first field ${js(form.first)}, focused ${form.focusIsFirst}; values empty ${form.password === '' && form.totp === ''}; Escape back to Edit ${!!back}`);
  });

  // 3. Nothing about the vault in the browser's storage.
  await check('IndexedDB, localStorage and sessionStorage hold no value and no vault entry name', async () => {
    await openVault();
    const dump = await evaluate(`(async () => {
      const out = { local: { ...localStorage }, session: { ...sessionStorage }, idb: {} };
      for (const { name } of await indexedDB.databases()) {
        const db = await new Promise((resolve, reject) => { const open = indexedDB.open(name); open.onsuccess = () => resolve(open.result); open.onerror = () => reject(open.error); });
        out.idb[name] = {};
        for (const store of db.objectStoreNames) {
          out.idb[name][store] = await new Promise((resolve) => { const request = db.transaction(store).objectStore(store).getAll(); request.onsuccess = () => resolve(request.result); request.onerror = () => resolve('unreadable'); });
        }
        db.close();
      }
      return JSON.stringify(out);
    })()`);
    const inStorage = found(dump, all);
    const named = Object.values(names).filter((name) => dump.includes(name));
    const stores = Object.entries(JSON.parse(dump).idb).map(([db, storesOf]) => `${db}(${Object.keys(storesOf).length})`);
    record('IndexedDB, localStorage and sessionStorage hold no value and no vault entry name',
      inStorage.length === 0 && named.length === 0 && dump.length > 2,
      `canaries ${inStorage.join(',') || 'none'}; names ${named.join(',') || 'none'}; databases ${stores.join(', ') || 'none'}; ${dump.length} bytes`);
  });

  // 4. Grants through a bruh's Settings.
  await check("Ash's Settings, Vault: the shared login's switch on and Save sends one set_vault; Pix's shows its own line", async () => {
    await openBruh('Ash', 'Settings');
    await waitFor("Ash's vault switches", `!!document.querySelector('[data-vault-grant="${entryIds.global}"]')`, 8000);
    const hint = await evaluate(`document.querySelector('#bruh-vault-hint')?.textContent.trim()`);
    const since = Date.now();
    await click(`[data-vault-grant="${entryIds.global}"]`);
    await waitFor('the switch is on', `document.querySelector('[data-vault-grant="${entryIds.global}"]').getAttribute('aria-checked') === 'true'`, 3000);
    await shoot('web-bruh-vault-ash-1280x800');
    await click('form[aria-label="Bruh settings"] button', 'Save');
    await until("Ash's grant is stored", async () => (await snapshot()).bruhs.find((state) => state.agent_id === ids.Ash)?.vault?.includes(entryIds.global), 5000).catch(() => null);
    await sleep(800);
    const setVault = commandsSince(since, 'set_vault');
    const others = sent.filter((entry) => entry.at >= since && entry.action !== 'set_vault').map((entry) => entry.action);
    const stored = (await snapshot()).bruhs.find((state) => state.agent_id === ids.Ash)?.vault;
    await openBruh('Pix', 'Settings');
    const own = await waitFor("Pix's own line", `[...document.querySelectorAll('[data-vault-own]')].map((node) => node.textContent.trim()).join('|') || null`, 8000).catch(() => null);
    const pixSwitches = await evaluate(`Object.fromEntries([...document.querySelectorAll('[data-vault-grant]')].map((node) => [node.getAttribute('data-vault-grant'), node.getAttribute('aria-checked')]))`);
    record("Ash's Settings, Vault: the shared login's switch on and Save sends one set_vault; Pix's shows its own line",
      hint === COPY.grantsHint && setVault.length === 1 && setVault[0].agent_id === ids.Ash && js(setVault[0].entries) === js([entryIds.global]) && others.length === 0
        && js(stored) === js([entryIds.global]) && own === `${names.pix}: its own` && pixSwitches[entryIds.key] === 'true' && pixSwitches[entryIds.op] === 'true' && pixSwitches[entryIds.global] === 'false',
      `hint ${js(hint)}; set_vault ${js(setVault.map(({ at, ...rest }) => rest))}; other commands ${others.join(',') || 'none'}; stored ${js(stored)}; Pix own ${js(own)}; Pix switches ${js(pixSwitches)}`);
  });

  // 5. Pix signs in with its login. Values reach the page and nothing else.
  const leakScan = (since, extra = '') => {
    const model = [...readLog('steps.log').filter((entry) => entry.at >= since), ...readLog('calls.log')].map((entry) => js(entry)).join('\n') + extra;
    return {
      model: found(model, all),
      log: found(serverLog(), all),
      stream: found(streamFrames.join(''), all),
      page: found(pageFrames.join('\n'), all),
      database: fileLeaks(databaseFiles(), all),
    };
  };
  const clean = (scan) => Object.values(scan).every((list) => list.length === 0);
  const scanText = (scan) => Object.entries(scan).map(([where, list]) => `${where} ${list.join(',') || 'none'}`).join('; ');
  await check("Pix: logins lists its login with no values; it fills the username and password with submit; the fixture gets both, and the reply, the stored run, the stream frames and the server log hold none", async () => {
    const since = Date.now();
    const { run } = await ask(ids.Pix, doLines(
      { action: 'logins' },
      { action: 'navigate', url: `${LOGIN}/in` },
      { action: 'fill', selector: '#email', secret: { login_id: entryIds.pix, field: 'username' } },
      { action: 'fill', selector: '#pw', secret: { login_id: entryIds.pix, field: 'password' }, submit: true },
    ));
    await until('the fixture gets the sign-in', async () => logged(ids.Pix, 'session', 'login.vault-fixture.test').length > 0, 8000).catch(() => null);
    await until('the code page loads', async () => logged(ids.Pix, 'page', 'login.vault-fixture.test').some((entry) => entry.path === '/code'), 8000).catch(() => null);
    const said = replies('Pix', since);
    let listed;
    try {
      listed = JSON.parse(said[0]).value.logins;
    } catch {
      listed = null;
    }
    const mine = listed?.find((login) => login.login_id === entryIds.pix);
    const session = logged(ids.Pix, 'session', 'login.vault-fixture.test').at(-1);
    const stored = (await snapshot()).runs.find((candidate) => candidate.id === run.id);
    const scan = leakScan(since, js(stored));
    record("Pix: logins lists its login with no values; it fills the username and password with submit; the fixture gets both, and the reply, the stored run, the stream frames and the server log hold none",
      run.status === 'completed' && !!mine && mine.origin === LOGIN && js(mine.fields) === js(['username', 'password', 'totp']) && !listed.some((login) => login.login_id === entryIds.global)
        && said[2]?.includes('"filled":"username"') && said[3]?.includes('"submitted":true') && session?.who === 'pix' && session.user && session.pass && clean(scan),
      `run ${run.status}; logins ${js(listed?.map((login) => ({ name: login.name, origin: login.origin, fields: login.fields })))}; fills ${js(said.slice(2))}; fixture sign-in ${js(session && { who: session.who, user: session.user, pass: session.pass })}; leaks: ${scanText(scan)}`);
  });

  // 6. The one-time code, made on the server at fill time.
  await check('Pix fills the one-time code on /code with submit, and the fixture accepts it', async () => {
    const since = Date.now();
    const { run } = await ask(ids.Pix, doLines({ action: 'fill', selector: '#code', secret: { login_id: entryIds.pix, field: 'totp' }, submit: true }));
    const verified = await until('the fixture checks the code', async () => logged(ids.Pix, 'verify').at(-1), 8000).catch(() => null);
    const said = replies('Pix', since);
    const scan = leakScan(since);
    record('Pix fills the one-time code on /code with submit, and the fixture accepts it',
      run.status === 'completed' && said[0]?.includes('"submitted":true') && verified?.ok === true && clean(scan),
      `run ${run.status}; reply ${js(said[0])}; fixture ${js(verified && { ok: verified.ok, length: verified.length })}; leaks: ${scanText(scan)}`);
  });

  // 7. After a fill: value selectors and screenshots are refused; a
  // snapshot works and holds no value, even with the password shown.
  await check('after a fill on the React-style page: value selectors are refused, an exact attribute works, a screenshot after Show password is refused, and a snapshot holds no value', async () => {
    const since = Date.now();
    const firstChar = canary.pixPass[0];
    const { run } = await ask(ids.Pix, doLines(
      { action: 'navigate', url: `${LOGIN}/in` },
      { action: 'fill', selector: '#pw', secret: { login_id: entryIds.pix, field: 'password' } },
      { action: 'click', selector: `input[value^="${firstChar}"]` },
      { action: 'click', selector: 'input[value]' },
      { action: 'click', selector: '#pw:placeholder-shown' },
      { action: 'click', selector: 'input:valid' },
      { action: 'click', selector: '[name=password]' },
      { action: 'click', selector: '#show' },
      { action: 'screenshot' },
      { action: 'snapshot' },
    ));
    const said = replies('Pix', since);
    const shown = logged(ids.Pix, 'show').filter((entry) => entry.at >= since).at(-1)?.value;
    const scan = leakScan(since);
    const probes = said.slice(2, 6);
    record('after a fill on the React-style page: value selectors are refused, an exact attribute works, a screenshot after Show password is refused, and a snapshot holds no value',
      run.status === 'completed' && said[1]?.includes('"filled":"password"') && probes.every((reply) => reply.includes(COPY.selector))
        && !said[6]?.startsWith('Error') && !said[6]?.startsWith('Denied') && shown === 'text'
        && said[8]?.includes(COPY.screenshot) && !said[9]?.startsWith('Error') && said[9]?.includes('Vault fixture sign in') && clean(scan),
      `fill ${js(said[1])}; probes ${js(probes.map((reply) => reply.includes(COPY.selector) ? 'refused' : reply.slice(0, 120)))}; [name=password] ${js(said[6]?.slice(0, 80))}; show ${shown}; screenshot ${js(said[8]?.slice(0, 120))}; snapshot ${js(said[9]?.slice(0, 80))}; leaks: ${scanText(scan)}`);
  });

  // 8. Another origin.
  await check('Pix fills its login on other.vault-fixture.test: LOGIN_WRONG_ORIGIN, and the page gets nothing', async () => {
    const since = Date.now();
    const { run } = await ask(ids.Pix, doLines(
      { action: 'navigate', url: `${OTHER}/in` },
      { action: 'fill', selector: '#email', secret: { login_id: entryIds.pix, field: 'username' } },
      { action: 'fill', selector: '#pw', secret: { login_id: entryIds.pix, field: 'password' }, submit: true },
    ));
    await sleep(1000);
    const said = replies('Pix', since);
    const typed = logged(ids.Pix, 'input', 'other.vault-fixture.test').filter((entry) => entry.at >= since);
    const posted = logged(ids.Pix, 'session', 'other.vault-fixture.test').filter((entry) => entry.at >= since);
    record('Pix fills its login on other.vault-fixture.test: LOGIN_WRONG_ORIGIN, and the page gets nothing',
      run.status === 'completed' && logged(ids.Pix, 'page', 'other.vault-fixture.test').some((entry) => entry.at >= since && entry.path === '/in')
        && said[1]?.includes(COPY.wrongOrigin) && said[2]?.includes(COPY.wrongOrigin) && typed.length === 0 && posted.length === 0,
      `replies ${js(said.slice(1).map((reply) => reply.slice(0, 100)))}; input events ${typed.length}; sign-ins ${posted.length}`);
  });

  // 9. Grants: Rex has none; Ash's granted login fills without a card,
  // and its next click still asks.
  await check("Rex fills Pix's login: LOGIN_NOT_GRANTED. Ash at Ask first fills its granted login with no approval card, and its next click asks", async () => {
    const rexSince = Date.now();
    const rex = await ask(ids.Rex, doLines(
      { action: 'navigate', url: `${LOGIN}/in` },
      { action: 'fill', selector: '#email', secret: { login_id: entryIds.pix, field: 'username' } },
    ));
    const rexSaid = replies('Rex', rexSince);
    const rexTyped = logged(ids.Rex, 'input').filter((entry) => entry.at >= rexSince);
    const ashSince = Date.now();
    const ash = await ask(ids.Ash, doLines(
      { action: 'navigate', url: `${LOGIN}/in` },
      { action: 'fill', selector: '#email', secret: { login_id: entryIds.global, field: 'username' } },
      { action: 'fill', selector: '#pw', secret: { login_id: entryIds.global, field: 'password' }, submit: true },
      { action: 'click', selector: '#code' },
    ), { decide: (item) => !js(item.approval).includes('#code') });
    await until("Ash's sign-in reaches the fixture", async () => logged(ids.Ash, 'session').length > 0, 5000).catch(() => null);
    const ashSaid = replies('Ash', ashSince);
    const session = logged(ids.Ash, 'session').at(-1);
    const cards = ash.approvals.map((item) => js(item.approval));
    const scan = leakScan(ashSince);
    record("Rex fills Pix's login: LOGIN_NOT_GRANTED. Ash at Ask first fills its granted login with no approval card, and its next click asks",
      rex.run.status === 'completed' && rexSaid[1]?.includes(COPY.notGranted) && rexTyped.length === 0
        && ash.run.status === 'completed' && ashSaid[1]?.includes('"filled":"username"') && ashSaid[2]?.includes('"submitted":true')
        && session?.who === 'global' && session.user && session.pass
        && cards.length === 2 && cards[0].includes(`${LOGIN}/in`) && cards[1].includes('#code') && ashSaid[3]?.startsWith('Denied') && clean(scan),
      `Rex ${js(rexSaid.map((reply) => reply.slice(0, 90)))}, typed ${rexTyped.length}; Ash replies ${js(ashSaid.map((reply) => reply.slice(0, 70)))}; Ash cards ${js(ash.approvals.map((item) => item.title + ': ' + (item.approval?.summary ?? '')))}; sign-in ${js(session && { who: session.who, user: session.user, pass: session.pass })}; leaks: ${scanText(scan)}`);
  });

  // 10. 1Password: signed out, then in.
  await check('the 1Password login signed out: ONE_PASSWORD_NEEDS_OWNER and the row says 1Password needs you; signed in, Check reads Ready. and the fill works', async () => {
    const since = Date.now();
    const out = await ask(ids.Pix, doLines(
      { action: 'navigate', url: `${OTHER}/in` },
      { action: 'fill', selector: '#email', secret: { login_id: entryIds.op, field: 'username' } },
    ));
    const outSaid = replies('Pix', since);
    await openVault();
    const row = `[data-vault-row="${entryIds.op}"]`;
    const attention = await waitFor('the attention line', `document.querySelector('${row} [data-vault-attention]')?.textContent.trim() ?? null`, 5000).catch(() => null);
    await shoot('web-vault-1password-needs-you-1280x800');
    fs.rmSync(signedOut, { force: true });
    await click(`${row} [data-vault-check]`);
    const checked = await waitFor('the check answers', `document.querySelector('${row} [data-vault-checked]')?.textContent.trim() ?? null`, 30000).catch(() => null);
    const attentionAfter = await evaluate(`document.querySelector('${row} [data-vault-attention]')?.textContent.trim() ?? null`);
    const inSince = Date.now();
    const signedIn = await ask(ids.Pix, doLines(
      { action: 'navigate', url: `${OTHER}/in` },
      { action: 'fill', selector: '#email', secret: { login_id: entryIds.op, field: 'username' } },
      { action: 'fill', selector: '#pw', secret: { login_id: entryIds.op, field: 'password' }, submit: true },
    ));
    await until('the fixture gets the 1Password sign-in', async () => logged(ids.Pix, 'session', 'other.vault-fixture.test').some((entry) => entry.at >= inSince), 8000).catch(() => null);
    const inSaid = replies('Pix', inSince);
    const session = logged(ids.Pix, 'session', 'other.vault-fixture.test').filter((entry) => entry.at >= inSince).at(-1);
    const reads = opCalls().filter((call) => call.at >= since).map((call) => call.reference);
    const text = await pageText();
    const scan = leakScan(since);
    record('the 1Password login signed out: ONE_PASSWORD_NEEDS_OWNER and the row says 1Password needs you; signed in, Check reads Ready. and the fill works',
      out.run.status === 'completed' && outSaid[1]?.includes(COPY.opOwner) && attention === COPY.opNeedsYou && checked === COPY.ready && attentionAfter === null
        && signedIn.run.status === 'completed' && inSaid[2]?.includes('"submitted":true') && session?.who === 'op' && session.user && session.pass
        && reads.includes(opRefs.username) && reads.includes(opRefs.password) && found(text).length === 0 && clean(scan),
      `signed out ${js(outSaid[1]?.slice(0, 100))}; row ${js(attention)}; Check ${js(checked)}, attention after ${js(attentionAfter)}; signed in ${js(inSaid.slice(1).map((reply) => reply.slice(0, 60)))}; sign-in ${js(session && { who: session.who, user: session.user, pass: session.pass })}; op reads ${reads.length}; leaks: ${scanText(scan)}`);
  });

  // 11. The keychain, 1Password and the vault's folder, from Pix's tools.
  await check("Pix's security and op shell lines are refused with VAULT_COMMAND_DENIAL, and a Read of credentials/vault.json through a link in its folder with VAULT_FILE_DENIAL", async () => {
    const work = path.join(tree(ids.Pix), 'work');
    const link = path.join(work, `notes-${tag}.json`);
    fs.mkdirSync(work, { recursive: true });
    fs.symlinkSync(path.join(credentials, 'vault.json'), link);
    const since = Date.now();
    const { run } = await ask(ids.Pix, doLines(
      'do-bash: security find-generic-password -s bruhs-ai-secrets -w',
      'do-bash: sh -c "op read op://P/I/password"',
      `do-read: ${link}`,
    ));
    const said = replies('Pix', since);
    record("Pix's security and op shell lines are refused with VAULT_COMMAND_DENIAL, and a Read of credentials/vault.json through a link in its folder with VAULT_FILE_DENIAL",
      run.status === 'completed' && said[0] === `Denied: ${COPY.command}` && said[1] === `Denied: ${COPY.command}` && said[2] === `Denied: ${COPY.file}`,
      `replies ${js(said)}`);
  });

  // 12. A bruh's key: every use asks, on its approval card.
  await check('Pix asks for DEPLOY_KEY: the card reads "Use DEPLOY_KEY as $DEPLOY_TOKEN in …"; Approve runs it (exit 0), a second request asks again, and no credential form shows', async () => {
    await openBruh('Pix', 'Chat');
    const since = Date.now();
    const before = (await runsOf(ids.Pix)).map((run) => run.id);
    await command({ action: 'chat', agent_id: ids.Pix, message: doLines(`do-secret: ${js(keyRequest)}`, `do-secret: ${js(keyRequest)}`) });
    const run = await newRun(ids.Pix, before);
    const cardText = `(() => {
      const card = [...document.querySelectorAll('article[aria-label="Approval"]')].find((node) => [...node.querySelectorAll('button')].some((button) => button.textContent.trim() === 'Approve' && !button.disabled));
      return card ? card.textContent.trim() : null;
    })()`;
    const first = await waitFor('the first card', cardText, 15000);
    const form = await evaluate(`!!document.querySelector('section[aria-label="Private credential request"]')`);
    await shoot('web-bruh-key-card-1280x800');
    await click('article[aria-label="Approval"] button', 'Approve');
    await until('the first use ends', async () => replies('Pix', since).length >= 1, 30000);
    const second = await waitFor('the second card', cardText, 15000).catch(() => null);
    const formAgain = await evaluate(`!!document.querySelector('section[aria-label="Private credential request"]')`);
    if (!second) {
      const open = (await snapshot()).outbox.filter((item) => item.run_id === run.id).map((item) => ({ title: item.title, resolved: !!item.resolved_at, approval: !!item.approval }));
      const shown = await evaluate(`[...document.querySelectorAll('article[aria-label="Approval"]')].map((node) => node.textContent.trim().slice(0, 120) + ' [' + [...node.querySelectorAll('button')].map((button) => button.textContent.trim() + (button.disabled ? ' off' : '')).join(',') + ']')`);
      await openBruh('Pix', 'Chat');
      const afterReload = await waitFor('the second card after a reload', cardText, 8000).catch(() => null);
      throw new Error(`no second card; outbox ${js(open)}; cards ${js(shown)}; after a reload ${js(afterReload?.slice(0, 80))}; replies ${js(replies('Pix', since))}`);
    }
    await click('article[aria-label="Approval"] button', 'Deny');
    const done = await until('the run ends', async () => {
      const found = (await snapshot()).runs.find((candidate) => candidate.id === run.id);
      return found && !live(found) ? found : null;
    }, 30000);
    const said = replies('Pix', since);
    let firstResult, secondResult;
    try {
      firstResult = JSON.parse(said[0].replace(/^Error: /, ''));
      secondResult = JSON.parse(said[1].replace(/^Error: /, ''));
    } catch {}
    const cards = (await snapshot()).outbox.filter((item) => item.run_id === run.id && item.approval);
    // The thread chat page has no credential form for a bruh's request either.
    await openApp('#/chat', '#chat-input');
    await sleep(800);
    const chatForm = await evaluate(`!!document.querySelector('section[aria-label="Private credential request"]')`);
    const scan = leakScan(since);
    const prefix = 'Use DEPLOY_KEY as $DEPLOY_TOKEN in ';
    record('Pix asks for DEPLOY_KEY: the card reads "Use DEPLOY_KEY as $DEPLOY_TOKEN in …"; Approve runs it (exit 0), a second request asks again, and no credential form shows',
      first.includes(prefix) && first.includes('Runs as you, outside the bruh') && firstResult?.status === 'completed' && firstResult.exit_code === 0
        && !!second && second.includes(prefix) && secondResult && secondResult.status !== 'completed' && cards.length === 2
        && !form && !formAgain && !chatForm && done.status === 'completed' && clean(scan),
      `first card ${js(first.slice(0, 160))}; first use ${js(firstResult)}; second card ${!!second}; second use ${js(secondResult)}; cards ${cards.length}; credential forms ${form}/${formAgain}/${chatForm}; run ${done.status}; leaks: ${scanText(scan)}`);
  });

  // 13. A thread's request finds the shared key.
  await check('a web thread asks for DEPLOY_KEY: its form says Saved for all projects with no Forget saved, and Use saved runs it', async () => {
    const thread = `vault-thread-${tag}`;
    await openApp('#/chat', '#chat-input');
    await evaluate(`(async () => {
      localStorage.setItem('bruhs.model', 'claude-sonnet-4-6');
      localStorage.setItem('bruhs.permissionMode', 'default');
      const db = await new Promise((resolve, reject) => { const open = indexedDB.open('bruhs'); open.onsuccess = () => resolve(open.result); open.onerror = () => reject(open.error); });
      await new Promise((resolve, reject) => {
        const tx = db.transaction(['threads'], 'readwrite');
        tx.objectStore('threads').put({ id: ${js(thread)}, title: 'Vault thread check', project_name: 'project', project_path: ${js(project)}, model: 'claude-sonnet-4-6', updated_at: String(Date.now()), pinned: false, settled: false, run_locked: true });
        tx.oncomplete = resolve; tx.onabort = () => reject(tx.error);
      });
      db.close();
    })()`);
    await openApp('#/chat', '#chat-input');
    await waitFor('the thread in the list', `!!document.querySelector('button[data-thread-id="${thread}"]')`, 20000);
    await click(`button[data-thread-id="${thread}"]`);
    await waitFor('the composer', `!!document.querySelector('#chat-input')`, 10000);
    await evaluate(`document.querySelector('#chat-input').focus()`);
    const since = Date.now();
    await type(`do-secret: ${js(keyRequest)}`);
    await press('Enter');
    const card = 'section[aria-label="Private credential request"]';
    const form = await waitFor('the credential form', `(() => { const card = document.querySelector('${card}'); return card ? { text: card.textContent.trim(), buttons: [...card.querySelectorAll('button')].map((button) => button.textContent.trim()) } : null; })()`, 20000)
      .catch(async () => { throw new Error(`no credential form; turn started ${readLog('calls.log').some((entry) => entry.name === 'thread' && entry.at >= since)}; composer ${js(await evaluate(`document.querySelector('#chat-input')?.value ?? null`))}; conversation ${js(sanitize(await evaluate(`document.querySelector('[role="log"]')?.textContent.slice(-300) ?? document.body.innerText.slice(0, 300)`)))}`); });
    await shoot('web-thread-key-form-1280x800');
    await click(`${card} button`, 'Use saved');
    await waitFor('the form closes', `!document.querySelector('${card}')`, 15000);
    const reply = await until("the thread's reply", async () => steps('thread').find((entry) => entry.phase === 'reply' && entry.at >= since), 30000);
    let result;
    try {
      result = JSON.parse(reply.reply.replace(/^Error: /, ''));
    } catch {}
    await sleep(1500);
    const text = await pageText();
    const scan = leakScan(since);
    const sse = bodies.filter((entry) => new URL(entry.url).pathname === '/api/agent').map((entry) => entry.body).join('\n');
    record('a web thread asks for DEPLOY_KEY: its form says Saved for all projects with no Forget saved, and Use saved runs it',
      form.text.includes('Saved for all projects') && form.buttons.includes('Use saved') && !form.buttons.includes('Forget saved')
        && result?.status === 'completed' && result.exit_code === 0 && found(text, all).length === 0 && found(sse, all).length === 0 && clean(scan),
      `request ${js(sse.split('\n').find((line) => line.includes('saved_available'))?.replace(/"command":\[[^\]]*\]/, '"command":[…]').replace(/"cwd":"[^"]*"/, '"cwd":…').slice(0, 400) ?? 'none')}; form buttons ${js(form.buttons)}; source ${form.text.includes('Saved for all projects')}; result ${js(result)}; values in the page ${found(text, all).join(',') || 'none'}, in the turn's SSE ${found(sse, all).join(',') || 'none'} (${sse.length} bytes); leaks: ${scanText(scan)}`);
  });
  // Storage again, after a thread used the vault: still no value.
  await check('after the thread: IndexedDB, localStorage and sessionStorage still hold no value', async () => {
    const dump = await evaluate(`(async () => {
      const out = [JSON.stringify({ ...localStorage }), JSON.stringify({ ...sessionStorage })];
      for (const { name } of await indexedDB.databases()) {
        const db = await new Promise((resolve, reject) => { const open = indexedDB.open(name); open.onsuccess = () => resolve(open.result); open.onerror = () => reject(open.error); });
        for (const store of db.objectStoreNames) out.push(JSON.stringify(await new Promise((resolve) => { const request = db.transaction(store).objectStore(store).getAll(); request.onsuccess = () => resolve(request.result); request.onerror = () => resolve('unreadable'); })));
        db.close();
      }
      return out.join('\\n');
    })()`);
    const inStorage = found(dump, all);
    record('after the thread: IndexedDB, localStorage and sessionStorage still hold no value', inStorage.length === 0 && dump.includes('Vault thread check'),
      `canaries ${inStorage.join(',') || 'none'}; the thread is stored ${dump.includes('Vault thread check')}`);
  });

  // 14. Deleting Pix takes its own entry, and only that.
  await check('Delete Pix: its own login leaves the listing; the shared entries stay', async () => {
    await openApp('#/automations');
    await waitFor('the agent cards', `document.querySelectorAll('[data-agent-delete]').length >= 3`, 10000);
    await evaluate(`[...document.querySelectorAll('details')].filter((details) => details.querySelector('[data-agent-delete]')).forEach((details) => { details.open = true; })`);
    const marked = await evaluate(`(() => {
      const card = [...document.querySelectorAll('article')].find((node) => node.querySelector('[data-agent-delete]') && node.textContent.includes('Pix'));
      card?.querySelector('[data-agent-delete]')?.setAttribute('data-verify-delete', '');
      return !!card;
    })()`);
    if (!marked) throw new Error("Pix's card has no Delete agent");
    await click('[data-verify-delete]');
    await waitFor('the question', `document.querySelector('[data-agent-delete-confirm]')?.textContent.trim() ?? null`, 3000);
    await click('[data-agent-delete-yes]');
    await until('Pix is deleted', async () => !(await snapshot()).agents.some((agent) => agent.id === ids.Pix), 8000);
    const listing = await until("Pix's login leaves the vault", async () => {
      const next = await vault();
      return next.entries.some((entry) => entry.id === entryIds.pix) ? null : next;
    }, 20000).catch(() => null);
    const left = (listing?.entries ?? []).map((entry) => entry.id).sort();
    await openVault();
    const shown = (await rows()).map((row) => row.id).sort();
    const expected = [entryIds.global, entryIds.key, entryIds.op].sort();
    record('Delete Pix: its own login leaves the listing; the shared entries stay',
      !!listing && js(left) === js(expected) && js(shown) === js(expected) && !(listing.bruhs ?? []).some((bruh) => bruh.agent_id === ids.Pix),
      `listing ${js(left)}; rows ${js(shown)}; bruhs ${js(listing?.bruhs?.map((bruh) => bruh.name))}`);
  });

  // 15. Phone size, touch and reduced motion.
  await check('390×844 touch: Vault controls are at least 44px, no sideways scroll, and the New entry form fits and closes', async () => {
    await viewport(390, 844, true);
    await send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
    await openVault();
    if (await evaluate(`!!document.querySelector('div.fixed.inset-0.z-30')`)) {
      for (const kind of ['mousePressed', 'mouseReleased']) {
        await send('Input.dispatchMouseEvent', { type: kind, x: 385, y: 600, button: 'left', clickCount: 1 });
      }
      await waitFor('the drawer closes', `!document.querySelector('div.fixed.inset-0.z-30')`, 3000);
    }
    await click('[data-vault-new]');
    await waitFor('the form', `!!document.querySelector('[data-vault-form]')`, 3000);
    await sleep(400);
    const layout = await evaluate(`(() => {
      const section = document.querySelector('#vault');
      const controls = [...section.querySelectorAll('input:not([type="checkbox"]), button, select')].filter((node) => node.getBoundingClientRect().height > 0).map((node) => [node.labels?.[0]?.textContent.trim() || node.getAttribute('aria-label') || node.textContent.trim().slice(0, 30), Math.round(node.getBoundingClientRect().height), Math.round(node.getBoundingClientRect().left), Math.round(node.getBoundingClientRect().right)]);
      return { controls, width: innerWidth, sideways: document.scrollingElement.scrollWidth - innerWidth, coarse: matchMedia('(pointer: coarse)').matches };
    })()`);
    await shoot('web-vault-390x844-touch');
    await click('[data-vault-form] button', 'Cancel');
    const closed = await waitFor('the form closes', `!document.querySelector('[data-vault-form]')`, 3000).catch(() => false);
    await send('Emulation.setTouchEmulationEnabled', { enabled: false });
    await viewport(1280, 800);
    const small = layout.controls.filter(([, height]) => height < 44);
    const outside = layout.controls.filter(([, , left, right]) => left < 0 || right > layout.width);
    record('390×844 touch: Vault controls are at least 44px, no sideways scroll, and the New entry form fits and closes',
      layout.coarse && layout.controls.length >= 10 && small.length === 0 && outside.length === 0 && layout.sideways <= 0 && !!closed,
      `coarse ${layout.coarse}; controls ${layout.controls.length}; under 44px ${js(small)}; outside ${js(outside)}; sideways ${layout.sideways}; closed ${!!closed}`);
  });
  await check('reduced motion leaves no animation frames on Settings, Vault', async () => {
    await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
    await openVault();
    await click('[data-vault-new]');
    await waitFor('the form', `!!document.querySelector('[data-vault-form]')`, 3000);
    await sleep(800);
    await evaluate('window.__frames = 0');
    await sleep(1000);
    const frames = await evaluate('window.__frames');
    await click('[data-vault-form] button', 'Cancel');
    await send('Emulation.setEmulatedMedia', { features: [] });
    record('reduced motion leaves no animation frames on Settings, Vault', frames === 0, `frames in a second ${frames}`);
  });

  // The whole run: no value in a tool result, the server log, a stream
  // frame, an app response, the reactive database or a file the server
  // wrote. Bruh browser profiles are the bruh's Chrome, which the owner's
  // page typed into; they are listed, not failed.
  await sleep(1000);
  {
    const scan = leakScan(0);
    const responseLeaks = bodies.flatMap((entry) => found(entry.body).map((name) => `${new URL(entry.url).pathname}:${name}`));
    const profile = (file) => file.includes(`${path.sep}browser${path.sep}`) || file.includes(`${path.sep}browser-cache${path.sep}`);
    const written = fileLeaks(filesUnder(home).concat(filesUnder(path.join(runDir, 'bruhs'), profile)));
    const profiles = fileLeaks(filesUnder(path.join(runDir, 'bruhs')).filter(profile));
    record('across the run: no value in a tool result, the server log, a stream frame, an app response, the reactive database or a file the server wrote',
      clean(scan) && responseLeaks.length === 0 && written.length === 0 && streamFrames.length > 0,
      `${scanText(scan)}; responses ${responseLeaks.join(',') || 'none'} of ${bodies.length}; files ${written.join(',') || 'none'}; stream chunks ${streamFrames.length}; page frames ${pageFrames.length}; in bruh browser profiles (not failed) ${profiles.join(',') || 'none'}`);
  }
  record('no panics in the page', panics.length === 0, panics.join(' | '));
  record('screenshots saved', screenshots.length > 0 && screenshots.every((file) => fs.statSync(file).size > 0), screenshots.map((file) => path.basename(file)).join(', '));

  exitCode = results.every((result) => result.pass) && !only ? 0 : 1;
  if (only) console.log(`Partial run (BRUHS_VERIFY_ONLY=${only.source}).`);
  fs.writeFileSync(path.join(runDir, 'results.json'), sanitize(JSON.stringify({ when: new Date().toISOString(), base, results, screenshots, fixtureLog }, null, 2)));
  console.log(`Screenshots: ${shots}`);
  const failed = results.filter((result) => !result.pass).length;
  console.log(`${results.length - failed}/${results.length} checks passed.`);
  console.log(exitCode === 0 ? 'All vault checks passed.' : `${failed} vault check(s) failed.`);
} catch (error) {
  console.error(`FAIL ${sanitize(error.stack ?? error)}`);
  fs.writeFileSync(path.join(runDir, 'results.json'), sanitize(JSON.stringify({ when: new Date().toISOString(), results, error: String(error), fixtureLog }, null, 2)));
} finally {
  await cleanup();
}
process.exit(exitCode);
