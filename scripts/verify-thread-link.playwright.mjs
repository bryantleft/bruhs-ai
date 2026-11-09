// Opt-in live check of thread-to-thread messages in the web app: a real
// Claude turn in one thread messages another thread, the view follows the
// message, the other thread's agent answers back, a turn starts a new
// thread, and the two Settings rows take effect. Spends real tokens, so it
// is not in the gate.
//
//   cargo build -p bruhs-server
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43021 trunk build --dist /tmp/bruhs-thread-link-dist)
//   BRUHS_PLAYWRIGHT_PATH=/path/to/playwright \
//     node scripts/verify-thread-link.playwright.mjs "$PWD" /tmp/bruhs-thread-link-dist --live
//
// The app runs with a disposable HOME, database and browser profile. The
// provider CLI keeps the real HOME so its sign-in works. BRUHS_LIVE_CLAUDE_MODEL
// picks the model (default claude-haiku-4-5).
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdtemp, mkdir, realpath, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';

assert(process.argv.includes('--live'), 'Pass --live to opt into real provider calls');
assert(process.argv[2] && process.argv[3], 'Workspace root and an isolated WASM build directory are required');
const root = resolve(process.argv[2]);
const dist = resolve(process.argv[3]);
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const model = process.env.BRUHS_LIVE_CLAUDE_MODEL || 'claude-haiku-4-5';
const origin = 'http://127.0.0.1:43021';
const artifacts = await realpath(await mkdtemp(join(tmpdir(), 'bruhs-thread-link-')));
const testHome = join(artifacts, 'home');
await mkdir(testHome, { recursive: true, mode: 0o700 });

// The provider CLIs keep the real HOME (their sign-in); the app does not.
const wrapperDir = join(artifacts, 'bin');
await mkdir(wrapperDir, { mode: 0o700 });
const shellQuote = value => `'${value.replaceAll("'", "'\\''")}'`;
for (const provider of ['claude', 'codex']) {
  const installed = spawnSync('/usr/bin/which', [provider], { encoding: 'utf8' }).stdout.trim();
  if (!installed.startsWith('/')) continue;
  await writeFile(join(wrapperDir, provider), `#!/bin/sh\nexport HOME=${shellQuote(homedir())}\nexec ${shellQuote(installed)} "$@"\n`, { mode: 0o700 });
}
const env = {
  ...process.env,
  HOME: testHome,
  PATH: `${wrapperDir}:${process.env.PATH}`,
  BRUHS_SERVER_PORT: '43021',
  BRUHS_WEB_DIST: dist,
  BRUHS_MEETING_DB: join(artifacts, 'meetings.sqlite3'),
  BRUHS_CAPTURE_URL: 'http://127.0.0.1:9',
  BRUHS_CAPTURE_TOKEN_FILE: join(artifacts, 'no-capture-token'),
  RUST_LOG: 'warn',
};
for (const name of ['BRUHS_API_URL', 'BRUHS_WEB_ORIGIN', 'ANTHROPIC_API_KEY', 'BRUHS_CLAUDE_USE_API_KEY']) delete env[name];

const report = { artifacts, origin, model, checks: [], turns: [], threadRequests: [], pageErrors: [] };
const check = (name, detail = {}) => {
  report.checks.push({ name, passed: true, ...detail });
  console.error(JSON.stringify({ check: name, ...detail }));
};
const waitFor = async (test, description, timeout = 180000) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await test()) return;
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  throw new Error(`Timed out: ${description}`);
};

const sender = randomUUID();
const research = randomUUID();
const short = id => id.slice(0, 8);
let server, browser, context, page, serverLog = '';

/// Every `/api/agent` turn the page started, in order, with its events.
const turnFor = thread => report.turns.filter(turn => turn.thread_id === thread);
const finished = turn => turn.events.some(event => event.type === 'result' || event.type === 'error');
const activeThread = () => page.evaluate(() => document.querySelector('button[data-thread-id][aria-current="true"], button[data-thread-id][aria-current="page"]')?.dataset.threadId ?? null);
const transcript = () => page.locator('main').innerText();
const settle = async () => {
  await page.locator('#chat-input').waitFor({ timeout: 45000 });
  await page.waitForFunction(() => !document.querySelector('[aria-label="Thread list loading status"]') && !document.querySelector('[aria-label="Conversation loading status"]'));
};

try {
  server = spawn(join(root, 'target/debug/bruhs-server'), [], { env, cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  server.stdout.on('data', value => { serverLog += value; });
  server.stderr.on('data', value => { serverLog += value; });
  await waitFor(async () => {
    if (server.exitCode !== null) throw new Error(`Isolated server exited: ${serverLog.slice(-800)}`);
    return fetch(origin + '/api/health').then(response => response.ok).catch(() => false);
  }, 'isolated server', 20000);
  const auth = await (await fetch(origin + '/api/agent/auth/claude', { headers: { Origin: origin } })).json();
  assert(auth.authenticated, 'Claude is not signed in for the isolated app');

  browser = await chromium.launch({ headless: true });
  context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  context.setDefaultTimeout(15000);
  await context.exposeBinding('__threadLinkEvent', (_source, { turn, event }) => {
    const record = report.turns[turn];
    if (!record) return;
    record.events.push(event);
    if (event.type === 'thread_request') report.threadRequests.push({ from: event.thread_id, operation: event.operation });
    if (['thread_request', 'result', 'error', 'permission_request'].includes(event.type)) {
      console.error(JSON.stringify({ turn, observed: event.type, operation: event.operation?.op, isError: event.is_error || false }));
    }
  });
  await context.exposeBinding('__threadLinkTurn', (_source, body) => {
    report.turns.push({ thread_id: body.thread_id, prompt: body.prompt, permission_mode: body.permission_mode, thread_access: body.thread_access, events: [] });
    return report.turns.length - 1;
  });
  await context.addInitScript(() => {
    const original = window.fetch;
    window.fetch = async (...args) => {
      const request = args[0] instanceof Request ? args[0] : null;
      const url = new URL(request ? request.url : String(args[0]), location.href);
      let turn = null;
      if (url.pathname === '/api/agent') {
        const body = request ? await request.clone().text() : String(args[1]?.body ?? '');
        turn = await window.__threadLinkTurn(JSON.parse(body));
      }
      const response = await original(...args);
      if (turn !== null && response.ok) {
        void (async () => {
          const reader = response.clone().body.getReader(), decoder = new TextDecoder();
          let buffer = '';
          while (true) {
            const { value, done } = await reader.read();
            if (done) break;
            buffer += decoder.decode(value, { stream: true });
            let index;
            while ((index = buffer.indexOf('\n\n')) >= 0) {
              const frame = buffer.slice(0, index);
              buffer = buffer.slice(index + 2);
              for (const line of frame.split('\n')) {
                if (line.startsWith('data:')) void window.__threadLinkEvent({ turn, event: JSON.parse(line.slice(5).trimStart()) });
              }
            }
          }
        })().catch(() => {});
      }
      return response;
    };
  });
  const replies = [];
  context.on('response', response => {
    if (new URL(response.url()).pathname === '/api/agent/thread-reply') replies.push(response.status());
  });
  page = await context.newPage();
  page.on('pageerror', error => report.pageErrors.push(String(error.stack || error)));

  // Two general threads, no history. Bypass, so nothing waits on an approval.
  await page.goto(origin + '/#/chat');
  await settle();
  await page.evaluate(async ({ sender, research, model }) => {
    localStorage.setItem('bruhs.model', model);
    localStorage.setItem('bruhs.permissionMode', 'bypassPermissions');
    const db = await new Promise((resolve, reject) => { const q = indexedDB.open('bruhs'); q.onsuccess = () => resolve(q.result); q.onerror = () => reject(q.error); });
    try {
      await new Promise((resolve, reject) => {
        const tx = db.transaction(['threads'], 'readwrite');
        const row = (id, title, at) => ({ id, title, project_name: '', model, updated_at: String(at), pinned: false, settled: false, run_locked: true });
        tx.objectStore('threads').put(row(research, 'Research', Date.now() - 1000));
        tx.objectStore('threads').put(row(sender, 'Sender', Date.now()));
        tx.oncomplete = resolve;
        tx.onabort = () => reject(tx.error);
      });
    } finally { db.close(); }
  }, { sender, research, model });
  await page.reload();
  await settle();

  // Settings: raise the chain limit, which every delivery header then names.
  const choose = async (label, option) => {
    await page.goto(origin + '/#/settings');
    const control = page.getByRole('button', { name: label, exact: true });
    await control.waitFor({ timeout: 45000 });
    await control.click();
    await page.getByRole('option', { name: option, exact: true }).click();
    await page.waitForFunction(({ label, option }) => document.querySelector(`button[aria-label="${label}"]`)?.innerText.includes(option), { label, option });
  };
  await choose('Agent message limit', '10 messages');
  assert.equal(await page.evaluate(() => localStorage.getItem('bruhs.threadHops')), '10');
  check('hop-limit-setting-saved');
  await page.goto(origin + '/#/chat');
  await settle();
  await page.locator(`button[data-thread-id="${sender}"]`).click();
  await settle();

  // 1. Sender lists the threads and messages Research.
  const word = `PLUM-${Math.floor(1000 + Math.random() * 9000)}`;
  await page.locator('#chat-input').fill(
    `This is a harness check of the bruhs_threads MCP tools. Call list_threads. Then call message_thread once, addressed to the thread titled "Research" (use its id from the list), with exactly this message: "Harness check. Call message_thread once, addressed to the thread id named in the header of this message, with exactly the message: ${word}. Then stop." Do not use any other tool. Then answer in one short line.`,
  );
  await page.locator('#chat-input').press('Enter');
  await waitFor(() => turnFor(sender).length === 1 && finished(turnFor(sender)[0]), 'the sender turn');
  const first = turnFor(sender)[0];
  assert(!first.events.some(event => event.type === 'error' || (event.type === 'result' && event.is_error)), 'The sender turn failed');
  const ops = report.threadRequests.filter(request => request.from === sender).map(request => request.operation);
  assert(ops.some(op => op.op === 'list'), 'The sender never listed threads');
  const sent = ops.find(op => op.op === 'message');
  assert(sent, 'The sender never messaged a thread');
  assert(research.startsWith(sent.thread_id), `The sender addressed ${sent.thread_id}, not Research`);
  const listed = first.events.find(event => event.type === 'tool_result' && event.content.includes('Threads, most recent first'));
  assert(listed && listed.content.includes(`${short(sender)} · Sender · no project · this thread`) && listed.content.includes(`${short(research)} · Research · no project · idle`), `Unexpected list: ${listed?.content}`);
  const delivered = first.events.find(event => event.type === 'tool_result' && event.content.includes('Sent to thread'));
  assert(delivered && delivered.content.includes(`${short(research)} "Research". It is queued`), `Unexpected delivery reply: ${delivered?.content}`);
  check('sender-listed-and-messaged', { operations: ops.map(op => op.op) });

  // 2. The view follows the message; Research runs it as its own turn.
  await waitFor(() => turnFor(research).length === 1, 'the Research turn to start');
  const second = turnFor(research)[0];
  assert(second.prompt.startsWith(`[Message from thread "Sender" (id ${short(sender)}), hop 1/10.`), `Unexpected delivered prompt: ${second.prompt.slice(0, 200)}`);
  assert(second.prompt.includes(word), 'The delivered prompt lost the message');
  // The transcript names the sender and hides the header written for the model.
  const caption = page.locator('[data-from-thread]').last();
  await caption.waitFor({ timeout: 20000 });
  assert.equal((await caption.innerText()).trim(), 'From thread \u201cSender\u201d');
  assert(!(await transcript()).includes('[Message from thread'), 'The model-facing header is shown to the user');
  assert((await transcript()).includes(word), 'The message body is missing from the transcript');
  check('view-followed-to-recipient', { prompt: second.prompt.split('\n')[0], caption: (await caption.innerText()).trim() });
  await page.screenshot({ path: join(artifacts, 'recipient-running.png') });
  await waitFor(() => finished(second), 'the Research turn');
  assert(!second.events.some(event => event.type === 'error' || (event.type === 'result' && event.is_error)), 'The Research turn failed');
  const answer = report.threadRequests.find(request => request.from === research && request.operation.op === 'message');
  assert(answer, 'Research never answered');
  assert(sender.startsWith(answer.operation.thread_id), `Research addressed ${answer.operation.thread_id}, not Sender`);
  assert(answer.operation.message.includes(word), 'Research answered without the word');
  check('recipient-answered', { message: answer.operation.message });

  // 3. The answer comes back as a new turn in Sender, one hop further.
  await waitFor(() => turnFor(sender).length === 2, 'the answer to start a turn in Sender');
  const third = turnFor(sender)[1];
  assert(third.prompt.startsWith(`[Message from thread "Research" (id ${short(research)}), hop 2/10.`), `Unexpected answer prompt: ${third.prompt.slice(0, 200)}`);
  assert(third.prompt.includes(word), 'The answer lost the word');
  await waitFor(() => finished(third), 'the answer turn');
  await waitFor(async () => (await transcript()).includes(word), 'the Sender transcript to show the answer', 20000);
  await page.screenshot({ path: join(artifacts, 'sender-after-answer.png') });
  check('answer-returned-to-sender', { prompt: third.prompt.split('\n')[0] });

  // 4. A turn reads another thread, then starts a new one.
  await waitFor(async () => !(await page.getByRole('button', { name: /^(Stop generation|Interrupt)$/ }).isVisible()), 'the app to go idle', 30000);
  const turnsBefore = report.turns.length;
  await page.locator('#chat-input').fill(
    `Harness check, second part. Call read_thread for the thread titled "Research" (id ${short(research)}) with limit 2. Then call message_thread once with no thread_id, title "Scout", and exactly this message: "Harness check. Reply to the user with the single word SCOUT-OK. Do not call any tool." Do not use any other tool. Then answer in one short line.`,
  );
  await page.locator('#chat-input').press('Enter');
  await waitFor(() => report.turns.length > turnsBefore && finished(report.turns[turnsBefore]), 'the second sender turn');
  const fourth = report.turns[turnsBefore];
  const read = fourth.events.find(event => event.type === 'tool_result' && event.content.startsWith('Thread "Research"'));
  assert(read && read.content.includes('[Message from thread "Sender"') && read.content.includes('Assistant:'), `Unexpected read: ${read?.content.slice(0, 300)}`);
  const started = fourth.events.find(event => event.type === 'tool_result' && event.content.startsWith('Started a new thread'));
  assert(started && started.content.includes('"Scout"'), `Unexpected new-thread reply: ${started?.content}`);
  check('sender-read-a-thread-and-started-one', { reply: started.content.split('.')[0] });
  await waitFor(() => report.turns.length > turnsBefore + 1, 'the Scout turn to start');
  const fifth = report.turns[turnsBefore + 1];
  assert(![sender, research].includes(fifth.thread_id), 'Scout did not get its own thread');
  assert(fifth.prompt.startsWith(`[Message from thread "Sender" (id ${short(sender)}), hop 1/10.`), `Unexpected Scout prompt: ${fifth.prompt.slice(0, 200)}`);
  await waitFor(() => finished(fifth), 'the Scout turn');
  await page.locator(`button[data-thread-id="${fifth.thread_id}"]`).waitFor();
  assert((await page.locator(`button[data-thread-id="${fifth.thread_id}"]`).innerText()).includes('Scout'), 'The new thread is not titled Scout');
  await waitFor(async () => (await transcript()).includes('SCOUT-OK'), 'the Scout transcript to show its reply', 20000);
  await page.screenshot({ path: join(artifacts, 'new-thread.png') });
  check('new-thread-ran', { thread: short(fifth.thread_id) });

  // 5. Read only in Settings: the next turn is sent with that access and is
  // offered the two read tools, not message_thread.
  await waitFor(async () => !(await page.getByRole('button', { name: /^(Stop generation|Interrupt)$/ }).isVisible()), 'the app to go idle', 30000);
  await choose('Messages between threads', 'Read only');
  assert.equal(await page.evaluate(() => localStorage.getItem('bruhs.threadAccess')), 'read');
  await page.goto(origin + '/#/chat');
  await settle();
  await page.locator(`button[data-thread-id="${sender}"]`).click();
  await settle();
  const before = report.turns.length;
  await page.locator('#chat-input').fill('Harness check, last part. Reply with the single word OK. Do not call any tool.');
  await page.locator('#chat-input').press('Enter');
  await waitFor(() => report.turns.length > before && finished(report.turns[before]), 'the read-only turn');
  const limited = report.turns[before];
  assert.equal(limited.thread_access, 'read', 'The turn was not sent with read-only access');
  const offered = limited.events.find(event => event.type === 'init')?.tools.filter(tool => tool.includes('bruhs_threads')) ?? [];
  assert.deepEqual(offered.sort(), ['mcp__bruhs_threads__list_threads', 'mcp__bruhs_threads__read_thread']);
  check('read-only-setting-limits-the-tools', { offered });

  assert(replies.length >= 5 && replies.every(status => status === 200), `Thread replies were not all accepted: ${replies}`);
  assert.equal(report.pageErrors.length, 0, `Browser runtime errors: ${report.pageErrors.join('\n')}`);
  check('all-answers-accepted', { answers: replies.length });
} catch (error) {
  report.error = String(error.stack || error);
  report.active = await activeThread().catch(() => null);
  await page?.screenshot({ path: join(artifacts, 'failure.png') }).catch(() => {});
  process.exitCode = 1;
} finally {
  try { await context?.close(); await browser?.close(); } catch { /* the browser already went away */ }
  if (server && server.exitCode === null) {
    server.kill('SIGTERM');
    await Promise.race([new Promise(resolve => server.once('exit', resolve)), new Promise(resolve => setTimeout(resolve, 10000))]);
    if (server.exitCode === null) server.kill('SIGKILL');
  }
  if (process.exitCode) report.serverLog = serverLog.slice(-2000);
  report.turns = report.turns.map(turn => ({ thread_id: turn.thread_id, prompt: turn.prompt.slice(0, 240), events: turn.events.map(event => event.type).join(',').slice(0, 400) }));
  await writeFile(join(artifacts, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
}
