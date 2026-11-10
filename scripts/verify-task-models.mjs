// Opt-in live check of the models by task: a meeting summary and both
// second-opinion reviews each run on the model the request names, and a
// model the job's provider cannot run is refused. Spends real tokens, so it
// is not in the gate.
//
//   cargo build -p bruhs-server
//   node scripts/verify-task-models.mjs "$PWD" --live
//
// The server runs with a disposable HOME and database. A stand-in capture
// daemon serves the transcript. The provider CLIs keep the real HOME so
// their sign-in works; each call's arguments are logged, which is how the
// check knows the model a job ran on. BRUHS_LIVE_CLAUDE_MODEL and
// BRUHS_LIVE_CODEX_MODEL pick the models (defaults claude-haiku-4-5 and
// gpt-6.1-sol).
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, mkdir, readFile, realpath, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';

assert(process.argv.includes('--live'), 'Pass --live to opt into real provider calls');
assert(process.argv[2], 'The workspace root is required');
const root = resolve(process.argv[2]);
const claudeModel = process.env.BRUHS_LIVE_CLAUDE_MODEL || 'claude-haiku-4-5';
const codexModel = process.env.BRUHS_LIVE_CODEX_MODEL || 'gpt-6.1-sol';
const origin = 'http://127.0.0.1:43022';
const artifacts = await realpath(await mkdtemp(join(tmpdir(), 'bruhs-task-models-')));
const testHome = join(artifacts, 'home');
await mkdir(testHome, { recursive: true, mode: 0o700 });

// Provider wrappers: the real HOME, and a line per call with its arguments.
const wrapperDir = join(artifacts, 'bin');
await mkdir(wrapperDir, { mode: 0o700 });
const shellQuote = value => `'${value.replaceAll("'", "'\\''")}'`;
const callLog = provider => join(artifacts, `${provider}-calls.log`);
const installed = {};
for (const provider of ['claude', 'codex']) {
  const path = spawnSync('/usr/bin/which', [provider], { encoding: 'utf8' }).stdout.trim();
  if (!path.startsWith('/')) continue;
  installed[provider] = true;
  await writeFile(callLog(provider), '');
  await writeFile(
    join(wrapperDir, provider),
    `#!/bin/sh\nprintf '%s\\n' "$*" >> ${shellQuote(callLog(provider))}\nexport HOME=${shellQuote(homedir())}\nexec ${shellQuote(path)} "$@"\n`,
    { mode: 0o700 },
  );
}
assert(installed.claude, 'The claude CLI is not installed');
const calls = async provider => (await readFile(callLog(provider), 'utf8')).split('\n').filter(Boolean);
const ranOn = (lines, model) => lines.some(line => line.split(/\s+/).includes(model) || line.includes(`=${model}`) || line.includes(`"${model}"`));

const report = { artifacts, origin, claudeModel, codexModel, checks: [] };
const check = (name, detail = {}) => {
  report.checks.push({ name, passed: true, ...detail });
  console.error(JSON.stringify({ check: name, ...detail }));
};

// The transcript the summary reads, served the way the capture daemon does.
const lines = [
  'Thanks for joining. Two things today: the launch date and the pricing page.',
  'On the launch date, we agreed to move it to the twelfth so QA gets a full week.',
  'For pricing, Dana will rewrite the page copy by Friday and send it around.',
  'One decision: we keep the free tier as it is for this release.',
];
const segmentsFor = meeting => lines.map((text, index) => ({
  id: `seg-${index}`,
  meeting_id: meeting,
  speaker_id: null,
  start_ms: index * 5000,
  end_ms: index * 5000 + 4000,
  text,
  confidence: 0.95,
  partial: false,
}));
const capture = createServer((request, response) => {
  const match = /^\/segments\/([A-Za-z0-9_-]+)$/.exec(new URL(request.url, 'http://capture').pathname);
  if (request.method === 'GET' && match) {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ segments: segmentsFor(match[1]) }));
    return;
  }
  response.writeHead(404, { 'content-type': 'application/json' });
  response.end('{}');
});
await new Promise(done => capture.listen(0, '127.0.0.1', done));
const tokenFile = join(artifacts, 'capture-token');
await writeFile(tokenFile, 'stand-in-token', { mode: 0o600 });

const env = {
  ...process.env,
  HOME: testHome,
  PATH: `${wrapperDir}:${process.env.PATH}`,
  BRUHS_SERVER_PORT: '43022',
  BRUHS_MEETING_DB: join(artifacts, 'meetings.sqlite3'),
  BRUHS_CAPTURE_URL: `http://127.0.0.1:${capture.address().port}`,
  BRUHS_CAPTURE_TOKEN_FILE: tokenFile,
  RUST_LOG: 'warn',
};
for (const name of ['BRUHS_API_URL', 'BRUHS_WEB_ORIGIN', 'BRUHS_WEB_DIST', 'ANTHROPIC_API_KEY', 'BRUHS_CLAUDE_USE_API_KEY']) delete env[name];

const api = async (method, path, body) => {
  const response = await fetch(origin + path, {
    method,
    headers: { Origin: origin, ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  return { status: response.status, json, text };
};

let server, serverLog = '';
try {
  server = spawn(join(root, 'target/debug/bruhs-server'), [], { env, cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  server.stdout.on('data', value => { serverLog += value; });
  server.stderr.on('data', value => { serverLog += value; });
  const deadline = Date.now() + 20000;
  for (;;) {
    if (server.exitCode !== null) throw new Error(`The isolated server exited: ${serverLog.slice(-800)}`);
    if (await fetch(origin + '/api/health').then(response => response.ok).catch(() => false)) break;
    if (Date.now() > deadline) throw new Error('Timed out: the isolated server');
    await new Promise(done => setTimeout(done, 250));
  }

  // 1. A meeting summary runs on the model the request names.
  const created = await api('POST', '/api/meetings', { title: 'Launch sync', started_at_ms: Date.now() - 60000 });
  assert.equal(created.status, 200, `The meeting was not created: ${created.text}`);
  const meeting = created.json.id;
  const stopped = await api('POST', `/api/meetings/${meeting}/stop`, { ended_at_ms: Date.now() });
  assert(stopped.status < 300, `The meeting was not stopped: ${stopped.text}`);

  const refused = await api('POST', `/api/meetings/${meeting}/summarize`, { extract_observations: false, model: codexModel });
  assert.equal(refused.status, 400, `A Codex model was not refused for a summary: ${refused.status} ${refused.text}`);
  assert.equal((await calls('claude')).length, 0, 'A refused summary still called Claude');
  check('summary-refuses-another-providers-model', { reply: refused.text.slice(0, 120) });

  const summary = await api('POST', `/api/meetings/${meeting}/summarize`, { extract_observations: false, model: claudeModel });
  assert.equal(summary.status, 200, `The summary failed: ${summary.status} ${summary.text.slice(0, 400)}`);
  assert(summary.json.tldr?.trim(), 'The summary has no tldr');
  const summaryCalls = await calls('claude');
  assert(ranOn(summaryCalls, claudeModel), `Claude was not called with ${claudeModel}: ${summaryCalls.join(' | ').slice(0, 400)}`);
  assert.equal(summary.json.generated_by, claudeModel, 'The summary does not record the model that wrote it');
  check('summary-ran-on-the-chosen-model', { model: summary.json.generated_by, tldr: summary.json.tldr.replace(/\s+/g, ' ').slice(0, 100) });

  // 2. A second opinion by Claude (on a Codex thread) runs on the chosen model.
  const task = 'Write a function that returns the larger of two integers.';
  const answer = 'fn max(a: i32, b: i32) -> i32 { if a > b { a } else { b } }';
  const wrong = await api('POST', '/api/agent/second-opinion', { thread_provider: 'codex', task, answer, reviewer_model: codexModel });
  assert.equal(wrong.status, 400, `A reviewer model of the wrong provider was not refused: ${wrong.status} ${wrong.text}`);
  check('review-refuses-the-other-providers-model', { reply: wrong.text.slice(0, 120) });

  const before = (await calls('claude')).length;
  const byClaude = await api('POST', '/api/agent/second-opinion', { thread_provider: 'codex', task, answer, focus: 'correctness', reviewer_model: claudeModel });
  assert.equal(byClaude.status, 200, `The Claude review failed: ${byClaude.status} ${byClaude.text.slice(0, 400)}`);
  assert.equal(byClaude.json.reviewer, 'claude');
  assert(!byClaude.json.error && byClaude.json.review.trim(), `The Claude review is empty: ${JSON.stringify(byClaude.json).slice(0, 300)}`);
  const reviewCalls = (await calls('claude')).slice(before);
  assert(ranOn(reviewCalls, claudeModel), `The Claude review was not called with ${claudeModel}: ${reviewCalls.join(' | ').slice(0, 400)}`);
  check('claude-review-ran-on-the-chosen-model', { model: claudeModel, review: byClaude.json.review.replace(/\s+/g, ' ').slice(0, 100) });

  // 3. A second opinion by Codex (on a Claude thread) runs on the chosen model.
  if (installed.codex) {
    const byCodex = await api('POST', '/api/agent/second-opinion', { thread_provider: 'claude', task, answer, focus: 'correctness', reviewer_model: codexModel });
    assert.equal(byCodex.status, 200, `The Codex review failed: ${byCodex.status} ${byCodex.text.slice(0, 400)}`);
    assert.equal(byCodex.json.reviewer, 'codex');
    const codexCalls = await calls('codex');
    assert(ranOn(codexCalls, codexModel), `Codex was not called with ${codexModel}: ${codexCalls.join(' | ').slice(0, 400)}`);
    assert(!byCodex.json.error && byCodex.json.review.trim(), `The Codex review is empty: ${JSON.stringify(byCodex.json).slice(0, 1500)}`);
    check('codex-review-ran-on-the-chosen-model', { model: codexModel, review: byCodex.json.review.replace(/\s+/g, ' ').slice(0, 100) });
  } else {
    report.skipped = 'The codex CLI is not installed, so the Codex review was not run.';
  }
} catch (error) {
  report.error = String(error.stack || error);
  process.exitCode = 1;
} finally {
  if (server && server.exitCode === null) {
    server.kill('SIGTERM');
    await Promise.race([new Promise(done => server.once('exit', done)), new Promise(done => setTimeout(done, 10000))]);
    if (server.exitCode === null) server.kill('SIGKILL');
  }
  capture.close();
  if (process.exitCode) report.serverLog = serverLog.slice(-2000);
  await writeFile(join(artifacts, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
}
