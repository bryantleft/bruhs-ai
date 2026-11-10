// Opt-in live check of thread-to-thread messages in the web app: a real
// Claude turn in one thread messages another thread, which runs it in the
// background and answers back; a turn waits for another thread's reply
// inside one tool call; a turn starts a new thread; the two Settings rows
// take effect; and one thread still stops and queues as before. Spends real
// tokens, so it is not in the gate.
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
const transcript = () => page.locator('main').innerText();
const stopButton = () => page.getByRole('button', { name: /^(Stop generation|Interrupt)$/ });
const idle = () => waitFor(async () => !(await stopButton().isVisible()), 'the thread on screen to go idle', 60000);
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
    if ((event.type === 'result' || event.type === 'error') && record.endedAt === null) record.endedAt = Date.now();
    if (event.type === 'thread_request') report.threadRequests.push({ from: event.thread_id, operation: event.operation });
    if (['thread_request', 'result', 'error', 'permission_request'].includes(event.type)) {
      console.error(JSON.stringify({ turn, observed: event.type, operation: event.operation?.op, isError: event.is_error || false }));
    }
  });
  await context.exposeBinding('__threadLinkTurn', (_source, body) => {
    report.turns.push({ thread_id: body.thread_id, prompt: body.prompt, permission_mode: body.permission_mode, thread_access: body.thread_access, events: [], startedAt: Date.now(), endedAt: null });
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
  assert(delivered && delivered.content.includes(`${short(research)} "Research". It is running now.`), `Unexpected delivery reply: ${delivered?.content}`);
  check('sender-listed-and-messaged', { operations: ops.map(op => op.op) });

  // 2. Research runs the message in the background; the view stays put.
  await waitFor(() => turnFor(research).length === 1, 'the Research turn to start');
  const second = turnFor(research)[0];
  assert(second.prompt.startsWith(`[Message from thread "Sender" (id ${short(sender)}), hop 1/10.`), `Unexpected delivered prompt: ${second.prompt.slice(0, 200)}`);
  assert(second.prompt.includes(word), 'The delivered prompt lost the message');
  assert((await transcript()).includes('harness check of the bruhs_threads MCP tools'), 'The view left the sender when its message started elsewhere');
  await waitFor(() => finished(second), 'the Research turn');
  assert(!second.events.some(event => event.type === 'error' || (event.type === 'result' && event.is_error)), 'The Research turn failed');
  const answer = report.threadRequests.find(request => request.from === research && request.operation.op === 'message');
  assert(answer, 'Research never answered');
  assert(sender.startsWith(answer.operation.thread_id), `Research addressed ${answer.operation.thread_id}, not Sender`);
  assert(answer.operation.message.includes(word), 'Research answered without the word');
  check('recipient-ran-in-the-background-and-answered', { prompt: second.prompt.split('\n')[0], message: answer.operation.message });

  // 3. The answer comes back as a new turn in Sender, one hop further.
  await waitFor(() => turnFor(sender).length === 2, 'the answer to start a turn in Sender');
  const third = turnFor(sender)[1];
  assert(third.prompt.startsWith(`[Message from thread "Research" (id ${short(research)}), hop 2/10.`), `Unexpected answer prompt: ${third.prompt.slice(0, 200)}`);
  assert(third.prompt.includes(word), 'The answer lost the word');
  await waitFor(() => finished(third), 'the answer turn');
  await waitFor(async () => (await transcript()).includes(word), 'the Sender transcript to show the answer', 20000);
  // The transcript names the sender and hides the header written for the model.
  const caption = page.locator('[data-from-thread]').last();
  await caption.waitFor({ timeout: 20000 });
  assert.equal((await caption.innerText()).trim(), 'From thread “Research”');
  assert(!(await transcript()).includes('[Message from thread'), 'The model-facing header is shown to the user');
  await page.screenshot({ path: join(artifacts, 'sender-after-answer.png') });
  check('answer-returned-to-sender', { prompt: third.prompt.split('\n')[0], caption: (await caption.innerText()).trim() });

  // Opening Research shows the turn it ran while it was off screen.
  await idle();
  await page.locator(`button[data-thread-id="${research}"]`).click();
  await settle();
  await waitFor(async () => (await transcript()).includes(word), 'the Research transcript to hold its background turn', 20000);
  assert.equal((await page.locator('[data-from-thread]').first().innerText()).trim(), 'From thread “Sender”');
  check('background-turn-was-saved');
  await page.locator(`button[data-thread-id="${sender}"]`).click();
  await settle();

  // 4. A turn waits, inside one tool call, for another thread's reply. The
  // two turns overlap: Research starts and ends while Sender's is held.
  const waitWord = `WAIT-${Math.floor(1000 + Math.random() * 9000)}`;
  const beforeWait = report.turns.length;
  await page.locator('#chat-input').fill(
    `Harness check of waiting. Call message_thread once, addressed to the thread titled "Research" (id ${short(research)}), with wait set to true and exactly this message: "Harness check. Reply with exactly ${waitWord} and nothing else. Do not call any tool." Do not use any other tool. Then tell me, in one short line, the exact text that tool call returned.`,
  );
  await page.locator('#chat-input').press('Enter');
  await waitFor(() => report.turns.length >= beforeWait + 2 && report.turns.slice(beforeWait).every(finished), 'the waiting turn and the turn it waited on', 240000);
  const waiting = report.turns.slice(beforeWait).find(turn => turn.thread_id === sender);
  const waitedOn = report.turns.slice(beforeWait).find(turn => turn.thread_id === research);
  assert(waiting && waitedOn, 'The waiting turn or the turn it waited on never ran');
  assert(waitedOn.prompt.includes('hop 1/10, awaiting your reply.'), `The recipient was not told its reply is awaited: ${waitedOn.prompt.slice(0, 200)}`);
  const returned = waiting.events.find(event => event.type === 'tool_result' && event.content.includes('answered:'));
  assert(returned && returned.content.includes(waitWord) && !returned.is_error, `The call did not return the reply: ${JSON.stringify(waiting.events.filter(event => event.type === 'tool_result').map(event => event.content.slice(0, 200)))}`);
  assert(waitedOn.startedAt > waiting.startedAt && waitedOn.endedAt < waiting.endedAt, 'The recipient did not run while the sender was held');
  assert(!report.threadRequests.some(request => request.from === research && request.operation.op === 'message' && request.operation.message.includes(waitWord)), 'The recipient messaged back although its reply was awaited');
  assert.equal(turnFor(sender).length, 3, 'The awaited reply also arrived as a message');
  await page.screenshot({ path: join(artifacts, 'sender-after-wait.png') });
  check('a-call-waited-for-another-threads-reply', { returned: returned.content.replace(/\s+/g, ' ').slice(0, 80), overlapMs: waitedOn.endedAt - waitedOn.startedAt });

  // 5. A turn reads another thread, then starts a new one, which runs in
  // the background.
  await idle();
  const turnsBefore = report.turns.length;
  await page.locator('#chat-input').fill(
    `Harness check, next part. Call read_thread for the thread titled "Research" (id ${short(research)}) with limit 2. Then call message_thread once with no thread_id, title "Scout", and exactly this message: "Harness check. Reply to the user with the single word SCOUT-OK. Do not call any tool." Do not set wait. Do not use any other tool. Then answer in one short line.`,
  );
  await page.locator('#chat-input').press('Enter');
  await waitFor(() => report.turns.length >= turnsBefore + 2 && report.turns.slice(turnsBefore).every(finished), 'the reading turn and the new thread');
  const fourth = report.turns.slice(turnsBefore).find(turn => turn.thread_id === sender);
  const fifth = report.turns.slice(turnsBefore).find(turn => turn.thread_id !== sender);
  const read = fourth.events.find(event => event.type === 'tool_result' && event.content.startsWith('Thread "Research"'));
  assert(read && read.content.includes('Assistant:') && read.content.includes(waitWord), `Unexpected read: ${read?.content.slice(0, 300)}`);
  const started = fourth.events.find(event => event.type === 'tool_result' && event.content.startsWith('Started a new thread'));
  assert(started && started.content.includes('"Scout"') && started.content.includes('It is running now.'), `Unexpected new-thread reply: ${started?.content}`);
  check('sender-read-a-thread-and-started-one', { reply: started.content.split('.')[0] });
  assert(![sender, research].includes(fifth.thread_id), 'Scout did not get its own thread');
  assert(fifth.prompt.startsWith(`[Message from thread "Sender" (id ${short(sender)}), hop 1/10.`), `Unexpected Scout prompt: ${fifth.prompt.slice(0, 200)}`);
  assert((await transcript()).includes('Harness check, next part'), 'The view left the sender when the new thread started');
  await page.locator(`button[data-thread-id="${fifth.thread_id}"]`).waitFor();
  assert((await page.locator(`button[data-thread-id="${fifth.thread_id}"]`).innerText()).includes('Scout'), 'The new thread is not titled Scout');
  await page.locator(`button[data-thread-id="${fifth.thread_id}"]`).click();
  await settle();
  await waitFor(async () => (await transcript()).includes('SCOUT-OK'), 'the Scout transcript to show its reply', 20000);
  await page.screenshot({ path: join(artifacts, 'new-thread.png') });
  check('new-thread-ran', { thread: short(fifth.thread_id) });

  // 6. One thread on its own still behaves: a prompt sent while its turn
  // runs is queued and runs next, and Stop ends a turn.
  await page.locator(`button[data-thread-id="${sender}"]`).click();
  await settle();
  await idle();
  const beforeQueue = report.turns.length;
  await page.locator('#chat-input').fill('Harness check of the queue. Count from 1 to 40, one number per line, and nothing else.');
  await page.locator('#chat-input').press('Enter');
  await waitFor(() => report.turns.length === beforeQueue + 1, 'the counting turn to start');
  await page.locator('#chat-input').fill('Reply with the single word QUEUED-OK. Do not call any tool.');
  await page.locator('#chat-input').press('Enter');
  assert.equal(report.turns.length, beforeQueue + 1, 'A prompt sent during a running turn started at once instead of queueing');
  await waitFor(() => report.turns.length === beforeQueue + 2 && finished(report.turns[beforeQueue + 1]), 'the queued prompt to run after the turn');
  assert(report.turns[beforeQueue + 1].startedAt >= report.turns[beforeQueue].endedAt, 'The queued prompt ran before the turn ahead of it ended');
  await waitFor(async () => (await transcript()).includes('QUEUED-OK'), 'the queued reply', 20000);
  check('a-prompt-sent-mid-turn-queued-and-ran-next');

  await idle();
  const beforeStop = report.turns.length;
  await page.locator('#chat-input').fill('Harness check of Stop. Write the numbers from 1 to 2000, one per line. Do not call any tool.');
  await page.locator('#chat-input').press('Enter');
  await waitFor(() => report.turns.length === beforeStop + 1 && report.turns[beforeStop].events.some(event => event.type === 'text'), 'the long turn to start writing');
  await stopButton().click();
  await idle();
  assert(!finished(report.turns[beforeStop]), 'The stopped turn ran to its end');
  assert.equal(await page.evaluate(() => localStorage.getItem('bruhs.promptQueue')), '[]', 'Something was left queued');
  // Stop pauses the queue; a send resumes it, and the thread still works.
  await page.locator('#chat-input').fill('Reply with the single word AFTER-STOP. Do not call any tool.');
  await page.locator('#chat-input').press('Enter');
  await waitFor(() => report.turns.length === beforeStop + 2 && finished(report.turns[beforeStop + 1]), 'a turn after Stop');
  await waitFor(async () => (await transcript()).includes('AFTER-STOP'), 'the reply after Stop', 20000);
  check('stop-ended-the-turn-and-the-thread-still-runs');

  // 7. Read only in Settings: the next turn is sent with that access and is
  // offered the two read tools, not message_thread.
  await idle();
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

  assert(replies.length >= 6 && replies.every(status => status === 200), `Thread replies were not all accepted: ${replies}`);
  assert.equal(report.pageErrors.length, 0, `Browser runtime errors: ${report.pageErrors.join('\n')}`);
  check('all-answers-accepted', { answers: replies.length });
} catch (error) {
  report.error = String(error.stack || error);
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
