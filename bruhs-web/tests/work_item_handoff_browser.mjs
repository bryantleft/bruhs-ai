// Real WASM, HTTP, task store, SDK processes and IndexedDB. The owned Codex
// fixture never contacts a model or executes copied prompt instructions.
// node ...mjs /fresh/dist /fresh/bruhs-server
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import { spawn, spawnSync } from 'node:child_process';
import { readFile, writeFile, mkdir, copyFile, chmod, stat, cp, symlink, unlink } from 'node:fs/promises';
import { homedir } from 'node:os';
import { createRequire } from 'node:module';
import { writeFileSync, appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { taskDeliveryChecks } from './task_delivery_checks.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
assert(process.argv[2] && process.argv[3], 'Pass a freshly built dist and server');
const dist = path.resolve(process.argv[2]), binary = path.resolve(process.argv[3]);
const live = process.argv.includes('--live');
const delivery = process.argv.includes('--delivery');
const effortChecks = process.argv.includes('--effort-checks');
assert(!effortChecks || !live, '--effort-checks uses the owned synthetic provider; use a separate runtime probe for live effort evidence');
const selectedModel = live ? 'codex/gpt-6.1-sol' : 'codex/fixture-model';
// Reuse the established Workbench 60s observation deadline and live coding
// pilot's declared 300s per-attempt budget, not production retry settings.
const timeoutMs = live ? 300000 : 60000;
const output = path.join(root, 'target/work-item-handoff', new Date().toISOString().replace(/[:.]/g, '-'));
await mkdir(output, { recursive: true });
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const sources = ['bruhs-models/src/workbench_handoff/work_item.rs', 'bruhs-models/src/coordination.rs', 'bruhs-core/src/coordination.rs', 'bruhs-core/src/coordination/handoffs.rs', 'bruhs-core/src/coordination/access.rs', 'bruhs-server/src/routes/coordination.rs', 'bruhs-server/src/routes/coordination/remote.html', 'bruhs-server/src/routes/workbench.html', 'bruhs-web/src/components/task_workbench.rs', 'bruhs-web/src/components/task_detail.rs', 'bruhs-web/src/components/workbench.rs', 'bruhs-web/src/state/workspace.rs', 'bruhs-web/src/task_saves/planning.rs'];
if (delivery) sources.push('bruhs-models/src/delivery.rs', 'bruhs-core/src/delivery.rs', 'bruhs-core/src/delivery/plan.rs', 'bruhs-core/src/workbench_tools.rs', 'bruhs-server/src/routes/delivery.rs', 'bruhs-server/src/routes/delivery_ui.html');
const fingerprints = await Promise.all(sources.map(async file => ({ file, sha256: sha(await readFile(path.join(root, file))), modifiedMs: (await stat(path.join(root, file))).mtimeMs })));
const webSources = fingerprints.filter(source => source.file.startsWith('bruhs-web/') || source.file.startsWith('bruhs-models/'));
assert((await stat(path.join(dist, 'index.html'))).mtimeMs >= Math.max(...webSources.map(source => source.modifiedMs)), 'Refuse stale WASM');
assert((await stat(binary)).mtimeMs >= Math.max(...fingerprints.filter(source => !source.file.startsWith('bruhs-web/')).map(source => source.modifiedMs)), 'Refuse stale server');
const home = path.join(output, 'home'), bin = path.join(home, 'bin'), workbench = path.join(output, 'workbench');
await mkdir(bin, { recursive: true });
const serverBinary = path.join(output, 'server-under-test');
await copyFile(binary, serverBinary);
const python = spawnSync('python3', ['-c', 'import sys; print(sys.executable)'], { encoding: 'utf8' }).stdout.trim();
assert(path.isAbsolute(python));
const provider = path.join(output, 'fixture_codex.py');
await copyFile(path.join(root, 'bench/workbench/fixture_codex.py'), provider);
const quote = text => `'${text.replaceAll("'", "'\\''")}'`;
await writeFile(path.join(bin, 'codex'), `#!/bin/sh\nexec ${quote(python)} ${quote(provider)} "$@"\n`, { mode: 0o700 });
await chmod(serverBinary, 0o700);
if (delivery) await writeFile(path.join(bin, 'gh'), '#!/bin/sh\necho "This acceptance run permits no forge commands" >&2\nexit 78\n', { mode: 0o700 });
const base = 'http://127.0.0.1:43023', calls = path.join(output, 'calls.jsonl');
let credentialLink;
const frozenFixture = path.join(output, 'frozen-paging-fixture');
async function prepareLiveProvider() {
  if (!live) return;
  await cp(path.join(root, 'bench/tasks/rootcause-paging'), frozenFixture, { recursive: true });
  const native = spawnSync('/usr/bin/which', ['codex'], { encoding: 'utf8' }).stdout.trim();
  assert(path.isAbsolute(native), 'Live acceptance requires installed Codex');
  const auth = path.join(process.env.CODEX_HOME || path.join(homedir(), '.codex'), 'auth.json');
  assert((await stat(auth)).isFile(), 'Live acceptance requires existing Codex sign-in');
  await mkdir(path.join(home, 'codex'), { recursive: true });
  credentialLink = path.join(home, 'codex/auth.json'); await symlink(auth, credentialLink);
  const wrapper = spawnSync(python, ['-c', 'import sys; from pathlib import Path; sys.path.insert(0,sys.argv[1]); from bench.live.run import provider_wrapper; provider_wrapper(Path(sys.argv[2]),Path(sys.argv[3]),Path(sys.argv[4]),"low")', root, native, path.join(bin, 'codex'), calls], { encoding: 'utf8' });
  assert.equal(wrapper.status, 0, wrapper.stderr);
}
const env = { HOME: home, XDG_DATA_HOME: path.join(home, 'data'), XDG_CONFIG_HOME: path.join(home, 'config'), CODEX_HOME: path.join(home, 'codex'), CLAUDE_CONFIG_DIR: path.join(home, 'claude'), PATH: `${bin}:/usr/bin:/bin`, BRUHS_SERVER_PORT: '43023', BRUHS_WEB_DIST: dist, BRUHS_WORKBENCH_DIR: workbench, BRUHS_DELIVERY_DIR: path.join(home, 'delivery'), BRUHS_MEETING_DB: path.join(home, 'meetings.sqlite3'), BRUHS_CAPTURE_URL: 'http://127.0.0.1:0', BRUHS_CAPTURE_TOKEN_FILE: path.join(home, 'missing-capture-token'), BRUHS_FIXTURE_LOG: calls, RUST_LOG: 'bruhs_server=info', GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null' };
const report = { output, dist, sources: fingerprints, serverSha256: sha(await readFile(serverBinary)), harnessSha256: sha(await readFile(fileURLToPath(import.meta.url))), checks: [], cells: [], live, selectedModel, effort: live ? 'low' : null, timeoutMs, scope: live ? 'One actual WASM task/planning/handoff/Workbench coding run with installed Codex GPT-6.1 Sol at low effort, a frozen internal paging fixture and independent pristine grading. Synthetic meeting/task inputs. No live capture/suggestion, competitor, statistically repeatable quality or PR-delivery claim.' : 'Actual web task, day planning, reviewed handoff, server/API/SDK/owned synthetic provider, isolated IndexedDB and durable host-store restart. Meeting and task records are synthetic. No model-quality, live suggestion, physical native or competitor evidence.' };
report.delivery = delivery;
report.effortChecks = effortChecks;
if (effortChecks) report.effort = 'high';
if (delivery) {
  report.deliveryHarnessSha256 = sha(await readFile(path.join(root, 'bruhs-web/tests/task_delivery_checks.mjs')));
  report.scope += ' Task delivery prepares a local proposal with retained source, checks removal and restart recovery, and executes no forge action.';
}
const check = (passed, label, actual) => { const entry = { label, passed: Boolean(passed), actual }; report.checks.push(entry); writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify({ label, passed: entry.passed })); if (!passed) throw new Error(label); };
const wait = async (test, label) => { const deadline = Date.now() + timeoutMs; while (Date.now() < deadline) { const value = await test(); if (value) return value; await new Promise(done => setTimeout(done, 100)); } throw new Error(`Timed out: ${label}`); };
async function api(method, endpoint, body) {
  const response = await fetch(base + endpoint, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined });
  const value = response.status === 204 ? null : await response.json();
  assert(response.ok, `${method} ${endpoint}: ${response.status} ${JSON.stringify(value)}`);
  return value;
}
async function invocations() { try { return (await readFile(calls, 'utf8')).trim().split('\n').filter(Boolean).map(JSON.parse); } catch (error) { if (error.code === 'ENOENT') return []; throw error; } }
let server, serverLog = '', browser, activePage;
async function startServer() {
  const offset = serverLog.length;
  server = spawn(serverBinary, [], { cwd: home, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  const log = bytes => { serverLog += bytes; appendFileSync(path.join(output, 'server.log'), bytes); };
  server.stdout.on('data', log); server.stderr.on('data', log);
  await wait(async () => {
    if (server.exitCode !== null) throw new Error('Owned server exited before readiness');
    if (!serverLog.slice(offset).includes('bruhs-server listening on http://127.0.0.1:43023')) return false;
    return (await fetch(base + '/api/coordination')).ok;
  }, 'owned server readiness');
}
async function stopServer() {
  if (!server || server.exitCode !== null || server.signalCode !== null) return;
  const child = server;
  child.kill('SIGINT');
  await wait(() => child.exitCode !== null || child.signalCode !== null, 'owned server exit');
}
async function records(page, seed) {
  return page.evaluate(seed => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs'); open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const db = open.result, tx = db.transaction(['work_items', 'events'], seed ? 'readwrite' : 'readonly');
      if (seed) for (const [name, values] of Object.entries(seed)) for (const value of values) tx.objectStore(name).put(value);
      const tasks = tx.objectStore('work_items').getAll(), events = tx.objectStore('events').getAll();
      tx.oncomplete = () => { db.close(); resolve({ tasks: tasks.result, events: events.result }); };
      tx.onabort = () => { db.close(); reject(tx.error); };
    };
  }), seed);
}
try {
  await prepareLiveProvider();
  const probe = createServer(); await new Promise((resolve, reject) => { probe.once('error', reject); probe.listen(43023, '127.0.0.1', resolve); }); await new Promise(done => probe.close(done));
  await startServer();
  const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
  browser = await chromium.launch({ headless: true }); report.browser = browser.version();
  const cells = [{ name: 'desktop', width: 1280, height: 900, touch: false }, ...live ? [] : [{ name: 'narrow', width: 390, height: 844, touch: true }]];
  report.plannedCells = cells;
  for (const cell of cells) {
    const state = { ...cell, checksStart: report.checks.length, errors: [] }; report.cells.push(state);
    const context = await browser.newContext({ viewport: { width: cell.width, height: cell.height }, hasTouch: cell.touch, timezoneId: 'America/Los_Angeles', serviceWorkers: 'block' });
    await context.addInitScript(({ model, effortChecks }) => {
      const NativeDate = Date, now = 1794844800000;
      globalThis.Date = new Proxy(NativeDate, { construct: (target, args) => Reflect.construct(target, args.length ? args : [now]), apply: () => new NativeDate(now).toString(), get: (target, key) => key === 'now' ? () => now : Reflect.get(target, key) });
      if (!localStorage.getItem('bruhs.model')) localStorage.setItem('bruhs.model', model);
      if (!localStorage.getItem('bruhs.permissionMode')) localStorage.setItem('bruhs.permissionMode', 'plan');
      if (effortChecks && !localStorage.getItem('bruhs.effort')) localStorage.setItem('bruhs.effort', 'high');
    }, { model: selectedModel, effortChecks });
    await context.route('**/*', route => { const url = new URL(route.request().url()); return url.origin === base ? route.continue() : route.abort(); });
    const page = await context.newPage(); activePage = page; page.setDefaultTimeout(12000); page.on('pageerror', error => state.errors.push(String(error)));
    const dismissSidebar = async () => { const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0').first(); if (await scrim.isVisible()) await scrim.click({ position: { x: cell.width - 10, y: 60 } }); };
    const project = path.join(home, cell.name); await mkdir(project);
    if (live) await cp(path.join(frozenFixture, 'repo'), project, { recursive: true });
    await writeFile(path.join(project, 'sentinel.txt'), 'Keep the original checkout intact.\n');
    for (const args of [['init', '-b', 'main'], ['add', '.'], ['-c', 'user.name=fixture', '-c', 'user.email=fixture@localhost', 'commit', '-m', 'baseline']]) assert.equal(spawnSync('/usr/bin/git', args, { cwd: project, env, encoding: 'utf8' }).status, 0);
    if (delivery) assert.equal(spawnSync('/usr/bin/git', ['remote', 'add', 'origin', 'https://github.com/fixture/repo.git'], { cwd: project, env, encoding: 'utf8' }).status, 0);
    const meeting = await api('POST', '/api/meetings', { title: `Synthetic ${cell.name} planning meeting`, started_at_ms: 1794844800000 });
    await api('PATCH', `/api/meetings/${meeting.id}`, { status: 'ended' });
    const description = live ? (await readFile(path.join(frozenFixture, 'prompt.md'), 'utf8')) + '\n\nWork directly without delegation for this declared single-run evaluation. Do not publish, merge or contact anyone.' : 'FIXTURE_CASE=success\nExercise the owned process fixture. Do not contact anyone or publish anything.';
    const task = { id: randomUUID(), project_id: null, title: `Reviewed ${cell.name} meeting action`, description, source: { kind: 'meeting', meeting_id: meeting.id, title: meeting.title }, status: 'Todo', priority: 'High', due_date: null, planning: { cycle: null, dependency_ids: [], estimate_minutes: 45, recurrence: null, time_block_event_ids: [], waiting_on: null }, created_at: 1794844800000, updated_at: 1794844800000 };
    await page.goto(base + '/#/today'); await page.getByRole('heading', { name: 'Today', exact: true }).waitFor();
    await page.waitForFunction(async () => (await indexedDB.databases()).some(db => db.name === 'bruhs'));
    await records(page, { work_items: [task] }); await page.reload();
    await page.getByRole('heading', { name: 'Today', exact: true }).waitFor(); await dismissSidebar();
    await page.getByText(task.title, { exact: true }).first().waitFor();
    await page.getByRole('button', { name: 'Plan my day', exact: true }).click();
    await page.getByRole('dialog', { name: 'Review day plan' }).getByRole('button', { name: 'Apply plan' }).click();
    await page.getByRole('button', { name: 'Undo', exact: true }).waitFor();
    const before = await records(page); state.before = before;
    check(before.tasks[0].planning.time_block_event_ids.length === 1 && before.events.length === 1, `${cell.name}: actual day plan is linked before handoff`, before);
    const openTask = async () => { await page.goto(base + '/#/tasks'); await page.getByText(task.title, { exact: true }).first().waitFor(); if (await page.locator('#task-detail-title').isVisible()) await page.locator('aside').filter({has:page.locator('#task-detail-title')}).getByTitle('Close',{exact:true}).click(); await dismissSidebar(); await page.getByText(task.title, { exact: true }).first().click(); await page.getByRole('button', { name: 'Review agent work', exact: true }).waitFor(); await wait(() => page.getByRole('button', { name: 'Review agent work', exact: true }).isEnabled(), 'task saves settle'); };
    await openTask();
    const reviewButton = page.getByRole('button', { name: 'Review agent work', exact: true }); await reviewButton.scrollIntoViewIfNeeded(); await reviewButton.focus();
    const [response] = await Promise.all([page.waitForResponse(response => new URL(response.url()).pathname === '/api/coordination/handoffs' && response.request().method() === 'POST'), page.keyboard.press(cell.touch ? 'Space' : 'Enter')]);
    const handoff = await response.json(); state.handoff = handoff;
    const frame = () => page.frameLocator('iframe[title="Local agent workbench"]').frameLocator('#panel');
    const capture = async name => {
      await wait(async()=>frame().locator('#drawer').evaluate(element=>!element.hidden && !element.classList.contains('entering') && getComputedStyle(element).opacity==='1'), 'drawer finishes opening');
      const geometry=await frame().locator('#drawer').evaluate(element=>{const b=element.getBoundingClientRect();return {x:b.x,y:b.y,width:b.width,height:b.height,viewportWidth:innerWidth,viewportHeight:innerHeight,opacity:getComputedStyle(element).opacity};});
      (state.frames ||= []).push({name,geometry});
      check(geometry.width>0 && geometry.height>0 && geometry.x>=-1 && geometry.x+geometry.width<=geometry.viewportWidth+1, `${cell.name}: ${name} is painted inside its frame`,geometry);
      await page.screenshot({path:path.join(output,`${cell.name}-${name}.png`)});
    };
    await frame().getByRole('heading', { name: 'Review agent work', exact: true }).waitFor();
    if (effortChecks) check(handoff.request.effort === 'high' && await frame().locator('#effort').inputValue() === 'high', `${cell.name}: saved preference reaches the task review`);
    await capture('review');
    check(await frame().locator('#project').evaluate(element=>document.activeElement===element), `${cell.name}: keyboard review begins in the required project field`);
    check(JSON.stringify(handoff.request.item) === JSON.stringify(before.tasks[0]), `${cell.name}: the complete saved task and plan are copied exactly`, handoff.request.item);
    check(handoff.request.model === selectedModel && handoff.request.permission_mode === 'plan' && (await api('GET', '/api/coordination')).tasks.length === report.cells.length - 1 && (await invocations()).length === report.cells.length - 1, `${cell.name}: preparation retains settings and runs nothing`, handoff.request);
    await page.reload(); await openTask();
    const [recovered] = await Promise.all([page.waitForResponse(response => new URL(response.url()).pathname === '/api/coordination/handoffs' && response.request().method() === 'POST'), page.getByRole('button', { name: 'Review agent work', exact: true }).click()]);
    check((await recovered.json()).id === handoff.id, `${cell.name}: app reload reopens the same durable handoff`);
    await frame().getByRole('heading', { name: 'Review agent work', exact: true }).waitFor();
    await frame().locator('#project').fill(path.join(project, 'missing-folder')); await frame().locator('#access').selectOption('acceptEdits'); await frame().locator('#isolation').selectOption('worktree');
    await frame().getByRole('button', { name: 'Start task', exact: true }).click();
    await wait(async () => (await frame().locator('#error').innerText()).length > 0, 'failed folder response');
    check((await api('GET', `/api/coordination/handoffs/${handoff.id}`)).handoff.started === null && JSON.stringify(await records(page)) === JSON.stringify(before), `${cell.name}: invalid Start preserves the source and remains reviewable`);
    await frame().locator('#project').fill(project);
    let reviewedRequest;
    const endpoint = `${base}/api/coordination/handoffs/${handoff.id}/start`;
    await context.route(endpoint, async route => { reviewedRequest = route.request().postDataJSON(); const response = await route.fetch(); assert(response.ok()); await route.abort('failed'); });
    await frame().getByRole('button', { name: 'Start task', exact: true }).click();
    await wait(async () => (await frame().locator('#error').innerText()).includes('Check start status'), 'lost response message'); await context.unroute(endpoint);
    const completed = await wait(async () => { const r = await api('GET', `/api/coordination/handoffs/${handoff.id}`); if (!r.handoff.started) return false; const t = (await api('GET', '/api/coordination')).tasks.find(t => t.id === r.handoff.started.task_id); return t?.state === 'completed' ? t : false; }, 'owned process completes');
    state.run = completed; state.reviewedRequest = reviewedRequest;
    await frame().getByRole('button', { name: 'Check start status', exact: true }).click();
    await frame().getByText('From personal task: ' + task.title, { exact: true }).waitFor();
    check(!(await frame().locator('#error').innerText()), `${cell.name}: a saved start receipt clears the lost-response error`);
    check(completed.work_item_origin.item_id === task.id && completed.work_item_origin.source.meeting_id === meeting.id && JSON.stringify(completed.work_item_origin.time_block_event_ids) === JSON.stringify(before.tasks[0].planning.time_block_event_ids), `${cell.name}: run retains typed task, meeting and plan identity`, completed.work_item_origin);
    if (effortChecks) {
      const call = (await invocations()).at(-1);
      check(completed.effort === 'high' && call.effort === 'high', `${cell.name}: reviewed effort reaches the SDK process`);
      const mismatched = await page.request.post(endpoint, { data: { ...reviewedRequest, effort: 'low' } });
      check(mismatched.status() === 409 && (await invocations()).length === report.cells.length, `${cell.name}: changed-effort Start retry cannot create another run`);
    }
    let outcome;
    if (live) {
      const grading = spawnSync(python, ['-c', 'import sys; from pathlib import Path; sys.path.insert(0,sys.argv[1]); from bench.live.run import grade; sys.exit(grade(Path(sys.argv[2]),Path(sys.argv[3]),Path(sys.argv[4]),float(sys.argv[5])))', root, frozenFixture, completed.working_path, path.join(output, 'independent-checks.log'), String(timeoutMs / 1000)], { encoding: 'utf8', timeout: timeoutMs + 1000 });
      state.grader = { exitCode: grading.status, error: grading.error?.message, stdout: grading.stdout, stderr: grading.stderr };
      outcome = grading.status === 0;
      state.usage = await api('GET', '/api/agent-extensions/usage');
    } else outcome = (await readFile(path.join(completed.working_path, 'outcome.txt'), 'utf8')) === completed.session_id;
    check(completed.permission_mode === 'acceptEdits' && completed.model === selectedModel && completed.working_path !== project && outcome, `${cell.name}: reviewed settings reach a settled SDK process and the independent outcome passes`, completed);
    await Promise.all([api('POST', `/api/coordination/handoffs/${handoff.id}/start`, reviewedRequest), api('POST', `/api/coordination/handoffs/${handoff.id}/start`, reviewedRequest)]);
    check((await invocations()).length === report.cells.length, `${cell.name}: concurrent retry after a lost response creates one provider invocation`);
    await stopServer(); await startServer();
    await api('POST', `/api/coordination/handoffs/${handoff.id}/start`, reviewedRequest);
    await page.reload(); await openTask();
    await page.getByRole('button', { name: 'Review agent work', exact: true }).click();
    await frame().getByText('From personal task: ' + task.title, { exact: true }).waitFor();
    if (effortChecks) {
      check(await frame().locator('#task-effort').inputValue() === 'high', `${cell.name}: restart restores requested effort`);
      await frame().locator('#task-effort').selectOption('low');
      await frame().getByRole('button', { name: 'Save effort', exact: true }).click();
      await wait(async () => (await api('GET', '/api/coordination')).tasks.find(t => t.id === completed.id).effort === 'low', 'effort update saves');
      check((await invocations()).length === report.cells.length && JSON.stringify(await records(page)) === JSON.stringify(before), `${cell.name}: saving effort starts no work and preserves personal data`);
      const saved = (await api('GET', '/api/coordination')).tasks.find(t => t.id === completed.id);
      check(saved.permission_mode === completed.permission_mode && saved.model === completed.model && saved.session_id === completed.session_id, `${cell.name}: effort update retains model, access and session`);
      await stopServer(); await startServer(); await page.reload(); await openTask();
      await page.getByRole('button', { name: 'Review agent work', exact: true }).click();
      await wait(async () => await frame().locator('#task-effort').inputValue() === 'low', 'saved effort restores after host restart');
      check((await invocations()).length === report.cells.length, `${cell.name}: saved effort survives restart without resuming the provider`);
    }
    check((await invocations()).length === report.cells.length && (await api('GET', '/api/coordination')).tasks.find(t => t.id === completed.id)?.state === 'completed', `${cell.name}: host restart retains the run and does not replay Start`);
    check(JSON.stringify(await records(page)) === JSON.stringify(before), `${cell.name}: completion, retry and host restart leave personal task and calendar unchanged`);
    await capture('completed');
    if (delivery) state.delivery = await taskDeliveryChecks({
      page, frame, task: completed, before, records, api, check, wait,
      restart: async () => { await stopServer(); await startServer(); },
      reopenRun: async () => { await page.reload(); await openTask(); await page.getByRole('button', { name: 'Review agent work', exact: true }).click(); await frame().getByText('From personal task: ' + task.title, { exact: true }).waitFor(); },
      output, name: cell.name, env,
    });
    await openTask();
    await page.locator('aside textarea').first().fill('Owner context before preparing another review.');
    await wait(async()=> (await records(page)).tasks[0].description === 'Owner context before preparing another review.' && await page.getByRole('button',{name:'Review agent work',exact:true}).isEnabled(), 'owner first edit saves');
    let copied, releaseCopy, heldCopy;
    const copyGate = new Promise(done=>{releaseCopy=done}), copyReady = new Promise(done=>{heldCopy=done});
    const prepareEndpoint=base+'/api/coordination/handoffs';
    await context.route(prepareEndpoint,async route=>{const response=await route.fetch();copied=await response.json();heldCopy();await copyGate;await route.fulfill({response});});
    await Promise.all([
      page.waitForResponse(response=>new URL(response.url()).pathname==='/api/coordination/handoffs'&&response.request().method()==='POST'),
      (async()=>{await page.getByRole('button',{name:'Review agent work',exact:true}).click();await copyReady;await page.locator('aside textarea').first().fill("Owner's newer context while preparing.");await wait(async()=> (await records(page)).tasks[0].description === "Owner's newer context while preparing.", 'newer owner edit saves');releaseCopy();})()
    ]);
    await context.unroute(prepareEndpoint);
    await page.evaluate(()=>new Promise(done=>requestAnimationFrame(()=>requestAnimationFrame(done))));
    check(new URL(page.url()).hash==='#/tasks' && (await records(page)).tasks[0].description === "Owner's newer context while preparing.",`${cell.name}: a late preparation response preserves the owner edit and view`);
    state.latePreparedCopy=copied;
    await page.goto(base+'/#/workbench');await frame().getByRole('button',{name:'Task handoffs',exact:true}).click();
    const copyRow=frame().locator('[data-handoff="'+copied.id+'"]');
    await copyRow.getByRole('button',{name:'Review',exact:true}).waitFor();
    await capture('retained-copies');
    await copyRow.getByRole('button',{name:'Discard',exact:true}).click();
    await copyRow.waitFor({state:'detached'});
    check((await fetch(base+'/api/coordination/handoffs/'+copied.id)).status===404,`${cell.name}: an unused prepared copy is discoverable and discardable`);
    const startedRow=frame().locator('[data-handoff="'+handoff.id+'"]');
    await startedRow.getByRole('button',{name:'Forget source copy',exact:true}).click();await startedRow.waitFor({state:'detached'});
    const forgotten=await fetch(base+`/api/coordination/handoffs/${handoff.id}/start`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(reviewedRequest)});
    const retained=(await api('GET','/api/coordination')).tasks.find(t=>t.id===completed.id);
    check(forgotten.status===404 && retained?.work_item_origin?.item_id===task.id && (await invocations()).length===report.cells.length,`${cell.name}: forgetting a copy preserves its run and makes the old Start link unusable`);
    check(JSON.stringify((await records(page)).events)===JSON.stringify(before.events),`${cell.name}: copy management preserves independent calendar data`);
    await capture('handoff-management');
    check(!state.errors.length, `${cell.name}: no browser runtime exception`, state.errors);
    state.checksEnd = report.checks.length; await context.close();
    await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  }
} catch (error) {
  report.terminalError = String(error.stack || error); process.exitCode = 1;
  if (activePage && !activePage.isClosed()) { await activePage.screenshot({ path: path.join(output, 'failure.png') }); report.failurePage = await activePage.locator('body').innerText(); }
}
finally {
  await browser?.close();
  try { await stopServer(); } catch (error) { report.cleanupError = String(error); if (server?.pid) { try { process.kill(-server.pid, 'SIGKILL'); } catch {} } process.exitCode = 1; }
  if (credentialLink) { await unlink(credentialLink); report.credentialLinkRemoved = true; }
  await writeFile(path.join(output, 'server.log'), serverLog);
  report.summary = { passed: report.checks.filter(c => c.passed).length, failed: report.checks.filter(c => !c.passed).length, completedCells: report.cells.filter(c => c.checksEnd).length };
  await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ ...report.summary, terminalError: report.terminalError, output }));
}
