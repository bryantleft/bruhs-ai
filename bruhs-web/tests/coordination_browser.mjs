import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const html = await readFile(new URL('../../bruhs-server/src/routes/coordination/remote.html', import.meta.url), 'utf8');
const task = {
  id: 'task-fixture', parent_id: null, relationship: 'root',
  title: '<img src=x onerror="window.fixtureXss=true">',
  project_path: '/synthetic/project', working_path: '/synthetic/project',
  model: 'claude-sonnet-4-5', permission_mode: 'default', spec_id: null,
  spec_revision: null, state: 'waiting_approval',
  messages: [{ role: 'user', text: 'Inspect the synthetic fixture', at: Date.now() }],
  live_output: 'Current output', error: null, session_id: null,
  approvals: [{ id: 'approval-fixture', tool: 'Bash', input: { command: 'echo synthetic' } }],
  created_at: Date.now(), updated_at: Date.now(), resume_at: null,
  auto_resume_on_limit: false, limit_reported: false,
};
const data = { can_control: true, revision: 1, tasks: [task], specs: [], guest_proposals: [] };
const commands = [];
const streams = new Set();
function publish() { data.revision++; for (const stream of streams) stream.write('event: refresh\ndata: refresh\n\n'); }
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://fixture');
  const guest = request.headers.authorization === 'Bearer guest-fixture';
  const send = (value, status = 200) => { response.writeHead(status, { 'content-type': 'application/json' }); response.end(JSON.stringify(value)); };
  if (url.pathname === '/' || url.pathname === '/api/coordination/ui') { response.writeHead(200, { 'content-type': 'text/html' }); response.end(html); return; }
  if (url.pathname === '/fixture/parent') { response.writeHead(200, { 'content-type': 'text/html' }); response.end('<!doctype html><title>Task parent fixture</title><iframe title="Tasks" src="/api/coordination/ui" style="width:100%;height:800px;border:0"></iframe>'); return; }
  if (url.pathname === '/manifest.webmanifest') { send({ name: 'Bruhs tasks', short_name: 'Bruhs', start_url: '/', display: 'standalone', icons: [{ src: '/icon-512.png', sizes: '512x512', type: 'image/png' }] }); return; }
  if (url.pathname === '/icon-512.png') { response.writeHead(200, { 'content-type': 'image/png' }); response.end(await readFile(new URL('../../bruhs-server/src/routes/coordination/icon-512.png', import.meta.url))); return; }
  if (url.pathname.endsWith('/changes')) { response.writeHead(200, { 'content-type': 'text/event-stream' }); streams.add(response); response.write('event: refresh\ndata: refresh\n\n'); request.on('close', () => streams.delete(response)); return; }
  if (request.method === 'GET' && url.pathname === '/fixture/state') { send({ commands, data }); return; }
  if (request.method === 'GET' && ['/api/coordination', '/api/tasks'].includes(url.pathname)) {
    send(guest ? { ...data, can_control: false, tasks: [{ ...task, working_path: '', project_path: '', approvals: [] }], specs: [], guest_proposals: [] } : data); return;
  }
  if (request.method === 'GET' && ['/api/coordination/board', '/api/board'].includes(url.pathname)) {
    const status = task.state === 'waiting_approval'
      ? { label: 'Needs approval', tone: 'action', busy: false }
      : { label: 'Working', tone: 'info', busy: true };
    send({ revision: data.revision, refreshing: false, link_error: null, cards: [{
      id: task.id, title: task.title, lane: 'building', status,
      project_path: guest ? '' : task.project_path, branch: null, pulls: [],
      model: task.model, model_label: task.model, relationship: 'root', parent_id: null,
      children: 0, active_children: 0, verifications: 0, verification: null,
      approvals: guest ? 0 : task.approvals.length, archived: false, updated_at: task.updated_at,
    }] }); return;
  }
  if (request.method === 'GET' && url.pathname === '/api/coordination/access') { send([]); return; }
  let body = ''; for await (const chunk of request) body += chunk;
  if (request.method === 'POST' && url.pathname === '/fixture/event') { task.live_output += JSON.parse(body || '{}').text || ' next'; publish(); send({ ok: true }); return; }
  if (request.method === 'POST' && url.pathname.endsWith('/commands')) {
    const command = JSON.parse(body); commands.push(command);
    if (guest) { send({ error: 'Guest cannot control tasks' }, 403); return; }
    if (command.action === 'approval') { task.approvals = []; task.state = 'running'; }
    publish(); send(data); return;
  }
  if (request.method === 'POST' && url.pathname === '/api/proposals') { commands.push({ guest: true, ...JSON.parse(body) }); send({ accepted: true }); return; }
  send({ error: 'Fixture route not found' }, 404);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
const base = `http://127.0.0.1:${address.port}`;
if (process.argv.includes('--serve')) {
  console.log(JSON.stringify({ base, local: `${base}/api/coordination/ui`, guestToken: 'guest-fixture' }));
} else {
  const require = createRequire(import.meta.url);
  const module = process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright';
  const { chromium } = require(module);
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${base}/api/coordination/ui#task=${task.id}`);
    await page.locator('#drawer-title').getByText(task.title, { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.fixtureXss), undefined);
    assert.equal(await page.locator('img').count(), 0);
    await page.locator('#next-message').fill('Keep this unfinished draft');
    await page.locator('#next-message').focus();
    await page.evaluate(() => fetch('/fixture/event', { method: 'POST', body: JSON.stringify({ text: ' streamed' }) }));
    await page.getByText('Current output streamed', { exact: true }).waitFor();
    assert.equal(await page.locator('#next-message').inputValue(), 'Keep this unfinished draft');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'next-message');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    await page.getByRole('button', { name: 'Allow once', exact: true }).click();
    await page.locator('#detail .status').getByText('Working', { exact: true }).waitFor();
    assert.equal(commands.some(command => command.action === 'approval' && command.request_id === 'approval-fixture'), true);
    await page.goto(base);
    await page.locator('#token').fill('guest-fixture');
    await page.getByRole('button', { name: 'Pair this device', exact: true }).click();
    await page.getByRole('button', { name: 'Propose message', exact: true }).waitFor();
    assert.equal(await page.getByRole('button', { name: 'Stop task and children', exact: true }).count(), 0);
    assert.equal(await page.locator('#create-pane').isVisible(), false);
    await page.locator('#guest-message').fill('Question for the owner');
    await page.getByRole('button', { name: 'Propose message', exact: true }).click();
    await page.getByText('Message sent to the owner for review.', { exact: true }).waitFor();
    assert.equal(commands.some(command => command.guest && command.text === 'Question for the owner'), true);
    await page.goto(`${base}/fixture/parent`);
    const frame = page.frameLocator('iframe');
    await frame.locator('#board').waitFor();
    const commandsBefore = commands.length;
    await page.evaluate(() => document.querySelector('iframe').contentWindow.postMessage({ type: 'bruhs:prefill-task', path: '/synthetic/issue', prompt: 'Fix issue 42', title: 'Issue 42' }, location.origin));
    await frame.locator('#prompt').waitFor({ state: 'visible' });
    assert.equal(await frame.locator('#prompt').inputValue(), 'Fix issue 42');
    assert.equal(await frame.locator('#project').inputValue(), '/synthetic/issue');
    assert.equal(commands.length, commandsBefore, 'Prefill must never execute a task');
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ passed: true, assertions: ['draft and focus survive streaming', 'script-like title is inert', 'mobile fits', 'approval reaches server', 'guest has proposal-only controls', 'embedded prefill starts no run'], source: fileURLToPath(new URL('../../bruhs-server/src/routes/coordination/remote.html', import.meta.url)) }));
  } finally { await browser.close(); for (const stream of streams) stream.end(); server.close(); }
}
