// QA-only: real shared Tasks UI + local HTTP. Provider text/protocol/usage is synthetic.
// The Python Campaign owner isolates HOME, blocks forge CLIs and nonprivate Git writes.
import assert from 'node:assert/strict';
import { readFile, open, rename } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';

const [input, output] = process.argv.slice(2);
const fixture = JSON.parse(await readFile(input, 'utf8'));
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const report = { passed: false, checks: [], profiles: [], page_errors: [], console: [], requests: [], resources: [], streams: [], request_failures: [],
  fixture_notices: ['The fixture repository has no origin and forge executables are blocked, so the board may show a pull-request lookup notice.', 'A /favicon.ico 404 is allowed because the minimal fixture dist supplies only the required real font.'],
  scope: 'Real local shared Tasks UI at desktop1440 and mobile390touch. All provider text, protocol, and usage are synthetic. No model quality, paid usage, or sandbox enforcement claim.',
  regression_scope: 'Board revision/status responses and polling flags are controlled synthetic transport adversaries. One changes response is intentionally suppressed to isolate board polling. Task snapshots, deadlines, files, sessions, process cleanup, and owner-controlled server restarts use the real local server. The reconnect case disables board timers to require subsequent live changes.' };
const atomicWrite = async (path, content) => {
  const temporary = `${path}.qa-${process.pid}-${Date.now()}`;
  const handle = await open(temporary, 'wx');
  try { await handle.writeFile(content); await handle.sync(); } finally { await handle.close(); }
  await rename(temporary, path);
};
const check = (name, actual, expected = true) => {
  const passed = JSON.stringify(actual) === JSON.stringify(expected);
  report.checks.push({ name, passed, actual, expected });
  assert.deepEqual(actual, expected, name);
};
const pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const calls = async () => existsSync(fixture.call_log) ? (await readFile(fixture.call_log, 'utf8')).split('\n').filter(Boolean).map(row => JSON.parse(row)).filter(row => row.role === 'provider') : [];
const api = async (path = '', body) => {
  const response = await fetch(`${fixture.base}/api/coordination${path}`, { method: body ? 'POST' : 'GET',
    headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(fixture.timeout_ms) });
  const value = await response.json();
  return { status: response.status, value };
};
const task = async id => (await api()).value.tasks.find(row => row.id === id);
const settled = async (id, expected) => {
  const deadline = Date.now() + fixture.timeout_ms;
  while (Date.now() < deadline) {
    const value = await task(id);
    if (value?.state === expected) return value;
    if (value && !['ready', 'running', 'waiting_approval', 'waiting_child'].includes(value.state)) throw new Error(`Task settled as ${value.state}, expected ${expected}: ${JSON.stringify(value)}`);
    await pause(50);
  }
  throw new Error(`Task ${id} did not become ${expected}`);
};
const activate = async (locator, profile) => {
  if (profile.hasTouch) await locator.tap();
  else { await locator.focus(); await locator.press('Enter'); }
};
const commandFrom = async (page, locator, profile) => {
  const pending = page.waitForResponse(response => response.url() === `${fixture.base}/api/coordination/commands` && response.request().method() === 'POST');
  await activate(locator, profile);
  const response = await pending;
  const value = await response.json();
  assert.equal(response.status(), 200, JSON.stringify(value));
  return value;
};
const screenshot = async (page, profile, label) => {
  check(`${profile.name}: ${label} fits outer viewport`, await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  check(`${profile.name}: ${label} detail content fits drawer`, await page.locator('.drawer-body').evaluate(node => node.scrollWidth <= node.clientWidth + 1));
  await atomicWrite(join(output, `${profile.name}-${label}.png`), await page.screenshot({ fullPage: true }));
  await atomicWrite(join(output, `${profile.name}-${label}.txt`), await page.locator('body').innerText());
};
const waitSettledControls = async page => page.waitForFunction(() => {
  const input = document.getElementById('task-budget');
  return input && !input.disabled && ![...document.querySelectorAll('#detail button')].find(button => button.textContent === 'Save run budget')?.disabled;
});
// Only board transport is adversarial below; tasks, snapshots, files and providers remain real.
const boardPath = '**/api/coordination/board';
const changesPath = '**/api/coordination/changes';
const staleBoard = (id, evidence, refreshing) => async route => {
  const response = await route.fetch();
  const board = await response.json();
  const card = board.cards.find(value => value.id === id);
  evidence.push({ real_revision: board.revision, real_status: card?.status, supplied_revision: 0,
    supplied_status: 'Working', refreshing, synthetic_transport_adversary: true });
  board.revision = 0;
  board.refreshing = refreshing;
  if (card) card.status = { label: 'Working', tone: 'info', busy: true };
  await route.fulfill({ response, contentType: 'application/json', body: JSON.stringify(board) });
};
const interruptDetail = async (page, name, draft) => {
  await page.waitForFunction(() => document.querySelector('#detail .status')?.textContent === 'Interrupted');
  await waitSettledControls(page);
  check(`${name}: detail shows Interrupted despite cached Working card`, await page.locator('#detail .status').innerText(), 'Interrupted');
  check(`${name}: detail retains budget error`, (await page.locator('#detail p.error').innerText()).includes('execution budget'));
  check(`${name}: detail retains partial text`, (await page.getByLabel('Task transcript', { exact: true }).innerText()).includes('SYNTHETIC fixture partial output'));
  check(`${name}: budget setter is available`, await page.getByRole('button', { name: 'Save run budget', exact: true }).isEnabled());
  check(`${name}: effort input is available`, await page.locator('#task-effort').isEnabled());
  check(`${name}: effort setter is available`, await page.getByRole('button', { name: 'Save effort', exact: true }).isEnabled());
  check(`${name}: unsent draft survives detail refresh`, await page.locator('#next-message').inputValue(), draft);
  check(`${name}: draft focus and text selection survive detail refresh`, await page.evaluate(() => ({
    id: document.activeElement.id, start: document.activeElement.selectionStart, end: document.activeElement.selectionEnd,
  })), { id: 'next-message', start: 5, end: 13 });
};
const focusDraft = async (page, draft) => {
  await page.locator('#next-message').fill(draft);
  await page.locator('#next-message').focus();
  await page.locator('#next-message').evaluate(node => node.setSelectionRange(5, 13));
};
const ownedRestart = async profile => {
  assert.ok(fixture.owner_control_request && fixture.owner_control_reply, 'Python Campaign owner must supply the restart handshake');
  const nonce = `${profile.name}-${Date.now()}`;
  await atomicWrite(fixture.owner_control_request, JSON.stringify({ operation: 'restart_owned_server', nonce, profile: profile.name }) + '\n');
  const deadline = Date.now() + fixture.timeout_ms * 2;
  while (Date.now() < deadline) {
    if (existsSync(fixture.owner_control_reply)) {
      const receipt = JSON.parse(await readFile(fixture.owner_control_reply, 'utf8'));
      if (receipt.nonce === nonce) { assert.equal(receipt.passed, true, JSON.stringify(receipt)); return receipt; }
    }
    await pause(50);
  }
  throw new Error('Owned server restart handshake did not finish');
};
const browser = await chromium.launch({ headless: true });
try {
  for (const profile of [
    { name: 'desktop1440', viewport: { width: 1440, height: 900 } },
    { name: 'mobile390touch', viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true },
  ]) {
    const row = { name: profile.name, passed: false, stale_board_responses: [], polling_board_responses: [] };
    let expectedOwnedRestart = false;
    let expectedNavigation = false;
    let browserSnapshotReads = 0;
    let browserChangeRequests = 0;
    const page = await browser.newPage(profile);
    page.setDefaultTimeout(fixture.timeout_ms);
    page.on('pageerror', error => report.page_errors.push({ profile: profile.name, message: error.message }));
    page.on('console', message => { if (['warning', 'error'].includes(message.type())) report.console.push({ profile: profile.name, type: message.type(), text: message.text(), location: message.location(), expected_owned_restart: expectedOwnedRestart }); });
    page.on('response', response => {
      if (response.status() >= 400 || new URL(response.url()).pathname.startsWith('/fonts/')) report.resources.push({ profile: profile.name, status: response.status(), url: response.url(), expected_owned_restart: expectedOwnedRestart });
      if (new URL(response.url()).pathname === '/api/coordination/changes') report.streams.push({ profile: profile.name, status: response.status(), sequence: browserChangeRequests });
    });
    page.on('requestfailed', request => report.request_failures.push({ profile: profile.name, url: request.url(), error: request.failure()?.errorText, expected_owned_restart: expectedOwnedRestart, expected_navigation: expectedNavigation }));
    page.on('request', request => {
      const pathname = new URL(request.url()).pathname;
      if (pathname === '/api/coordination' && request.method() === 'GET') browserSnapshotReads++;
      if (pathname === '/api/coordination/changes') browserChangeRequests++;
      if (request.url() === `${fixture.base}/api/coordination/commands` && request.method() === 'POST') report.requests.push({ profile: profile.name, body: request.postDataJSON() });
    });
    await page.route('**/*', route => {
      const url = new URL(route.request().url());
      if (url.origin === fixture.base) return route.continue();
      report.page_errors.push({ profile: profile.name, message: `Blocked unexpected external browser request to ${url.origin}` });
      return route.abort();
    });
    try {
      expectedNavigation = true;
      await page.goto(`${fixture.base}/api/coordination/ui`);
      expectedNavigation = false;
      await page.locator('#new-task').waitFor({ state: 'visible' });
      check(`${profile.name}: real local Monaspace font loads`, await page.evaluate(async () => {
        await document.fonts.ready;
        return document.fonts.check('14px MonaspaceNeonVariable') && [...document.fonts].some(font => font.family.includes('MonaspaceNeonVariable') && font.status === 'loaded');
      }));
      if (profile.hasTouch) await page.locator('#new-task').tap();
      else { await page.locator('#new-task').focus(); await page.keyboard.press('n'); }
      await page.locator('#execution-budget').waitFor({ state: 'visible' });
      const createForm = page.getByRole('form', { name: 'New task settings', exact: true });
      check(`${profile.name}: New task settings form is accessible by role and name`, await createForm.isVisible());
      await atomicWrite(join(output, `${profile.name}-create-form-aria.txt`), await createForm.ariaSnapshot());
      check(`${profile.name}: create budget has an accessible label`, await page.getByLabel('Run budget in seconds (optional)', { exact: true }).count(), 1);
      check(`${profile.name}: blank create budget means unlimited`, await page.locator('#execution-budget').inputValue(), '');
      const title = `SYNTHETIC ${profile.name} run ${'longtitle'.repeat(24)}`;
      row.title = title;
      await page.locator('#title').fill(title);
      await page.locator('#project').fill(fixture.project);
      await page.locator('#model').fill('codex/fixture-model');
      await page.locator('#effort').selectOption('high');
      await page.locator('#access').selectOption('acceptEdits');
      await page.locator('#isolation').selectOption('worktree');
      await page.locator('#base').fill('main');
      await page.locator('#prompt').fill('SYNTHETIC browser fixture only. BUDGET_FIXTURE_CASE=stall');
      await page.locator('#execution-budget').fill('0');
      const beforeInvalid = (await calls()).length;
      const requestCount = report.requests.length;
      await activate(page.locator('#create').getByRole('button', { name: 'Start task', exact: true }), profile);
      check(`${profile.name}: zero create budget triggers native positive-integer validation`, await page.locator('#execution-budget').evaluate(node => !node.validity.valid && node.validationMessage.length > 0));
      check(`${profile.name}: invalid create budget sends no command`, report.requests.length, requestCount);
      check(`${profile.name}: invalid create budget starts no fake provider`, (await calls()).length, beforeInvalid);
      await screenshot(page, profile, 'create-validation');
      await page.locator('#execution-budget').fill('5');
      const snapshot = await commandFrom(page, page.locator('#create').getByRole('button', { name: 'Start task', exact: true }), profile);
      const created = snapshot.tasks.find(item => item.title === title);
      assert.ok(created, 'Created real server task must appear in command response');
      row.id = created.id;
      check(`${profile.name}: create sends explicit budget`, created.execution_timeout_seconds, 5);
      check(`${profile.name}: create retains requested effort`, created.effort, 'high');
      await page.locator('#task-budget').waitFor({ state: 'visible' });
      await page.waitForFunction(() => document.getElementById('task-budget')?.disabled);
      check(`${profile.name}: active budget input disabled`, await page.locator('#task-budget').isDisabled());
      check(`${profile.name}: active save budget disabled`, await page.getByRole('button', { name: 'Save run budget', exact: true }).isDisabled());
      const refused = await api('/commands', { action: 'set_execution_budget', id: created.id, seconds: 8 });
      check(`${profile.name}: actual server refuses active budget edit`, refused.status, 409);
      await screenshot(page, profile, 'active');
      const staleHandler = staleBoard(created.id, row.stale_board_responses, false);
      await page.route(boardPath, staleHandler);
      const staleDraft = `SYNTHETIC unsent ${profile.name} draft retained across stale board data.`;
      await focusDraft(page, staleDraft);
      const interrupted = await settled(created.id, 'interrupted');
      await interruptDetail(page, `${profile.name}: stale board revision`, staleDraft);
      check(`${profile.name}: stale board transport was exercised`, row.stale_board_responses.length > 0);
      check(`${profile.name}: supplied board revision is older than fresh snapshot`, (await api()).value.revision > 0 && row.stale_board_responses.every(value => value.supplied_revision === 0));
      await screenshot(page, profile, 'stale-board-interrupted');
      await page.unroute(boardPath, staleHandler);
      await page.waitForFunction(() => document.querySelector('#detail .status')?.textContent.includes('Interrupted'));
      await waitSettledControls(page);
      check(`${profile.name}: Interrupted status is visible`, await page.locator('#detail .status').innerText(), 'Interrupted');
      check(`${profile.name}: actual partial text is visible`, (await page.getByLabel('Task transcript', { exact: true }).innerText()).includes('SYNTHETIC fixture partial output'));
      check(`${profile.name}: budget error is visible in task detail`, (await page.locator('#detail p.error').innerText()).includes('execution budget'));
      check(`${profile.name}: expired task has no quota continuation scheduled`, interrupted.resume_at, null);
      check(`${profile.name}: interrupted private partial file persists`, existsSync(join(interrupted.working_path, 'budget-partial.txt')));
      const afterTimeout = (await calls()).length;
      await pause(250);
      check(`${profile.name}: UI never automatically retries expired task`, (await calls()).length, afterTimeout);
      await screenshot(page, profile, 'interrupted');

      await page.locator('#task-budget').fill('0');
      await activate(page.getByRole('button', { name: 'Save run budget', exact: true }), profile);
      await page.waitForFunction(() => document.getElementById('error').textContent.includes('positive whole-number'));
      check(`${profile.name}: invalid settled budget yields role-alert feedback`, (await page.locator('#error[role="alert"]').innerText()).includes('positive whole-number'));
      check(`${profile.name}: invalid settled edit starts no provider`, (await calls()).length, afterTimeout);
      await screenshot(page, profile, 'settled-validation');
      const save = async budget => {
        await page.locator('#task-budget').fill(budget);
        await commandFrom(page, page.getByRole('button', { name: 'Save run budget', exact: true }), profile);
        await waitSettledControls(page);
        return task(created.id);
      };
      let saved = await save('7');
      check(`${profile.name}: edited budget persists on real server`, saved.execution_timeout_seconds, 7);
      check(`${profile.name}: edited budget appears in visible facts`, (await page.locator('#detail .facts').innerText()).includes('Run budget: 7 seconds'));
      for (const key of ['state', 'session_id', 'model', 'permission_mode', 'effort', 'messages', 'working_path']) check(`${profile.name}: saving budget preserves ${key}`, saved[key], interrupted[key]);
      saved = await save('');
      check(`${profile.name}: blank settled budget persists unlimited`, saved.execution_timeout_seconds ?? null, null);
      check(`${profile.name}: unlimited budget is visible in facts`, (await page.locator('#detail .facts').innerText()).includes('Run budget: unlimited'));
      saved = await save('7');
      check(`${profile.name}: all settled budget edits start no provider`, (await calls()).length, afterTimeout);
      check(`${profile.name}: existing effort control remains`, await page.locator('#task-effort').inputValue(), 'high');
      if (profile.hasTouch) {
        const sizes = await page.locator('#task-budget').evaluate(node => ({ width: node.getBoundingClientRect().width, height: node.getBoundingClientRect().height }));
        const saveHeight = await page.getByRole('button', { name: 'Save run budget', exact: true }).evaluate(node => node.getBoundingClientRect().height);
        check(`${profile.name}: budget and save have 44px touch targets`, sizes.height >= 44 && saveHeight >= 44);
        row.touch_targets = { budget: sizes, save_height: saveHeight };
      } else {
        await page.locator('#task-budget').focus();
        check(`${profile.name}: budget exposes a visible keyboard focus ring`, await page.locator('#task-budget').evaluate(node => getComputedStyle(node).outlineStyle !== 'none' && parseFloat(getComputedStyle(node).outlineWidth) >= 2));
        await page.keyboard.press('Tab');
        check(`${profile.name}: Tab reaches Save run budget from budget input`, await page.evaluate(() => document.activeElement.textContent), 'Save run budget');
      }
      await screenshot(page, profile, 'edited');
      await page.locator('#next-message').fill('SYNTHETIC owner explicitly resumes. BUDGET_FIXTURE_CASE=resume');
      await commandFrom(page, page.getByRole('button', { name: 'Send message', exact: true }), profile);
      const completed = await settled(created.id, 'completed');
      await page.waitForFunction(() => document.querySelector('#detail .transcript')?.textContent.includes('SYNTHETIC explicit completion.'));
      check(`${profile.name}: completed task is visibly settled`, ['Idle', 'Done'].includes(await page.locator('#detail .status').innerText()));
      check(`${profile.name}: explicit resume retains session`, completed.session_id, interrupted.session_id);
      check(`${profile.name}: explicit resume uses latest marker`, (await calls()).at(-1).case, 'resume');
      check(`${profile.name}: explicit resume is one new provider turn`, (await calls()).length, afterTimeout + 1);
      check(`${profile.name}: explicit resume completes private file`, existsSync(join(completed.working_path, 'budget-completed.txt')));
      check(`${profile.name}: synthetic final text is visible`, (await page.getByLabel('Task transcript', { exact: true }).innerText()).includes('SYNTHETIC explicit completion.'));
      await screenshot(page, profile, 'explicit-completion');
      const beforeReload = (await calls()).length;
      expectedNavigation = true;
      await page.reload();
      expectedNavigation = false;
      await page.locator(`[data-card="${created.id}"]`).waitFor();
      await activate(page.locator(`[data-card="${created.id}"]`), profile);
      await page.locator('#task-budget').waitFor({ state: 'visible' });
      check(`${profile.name}: reload retains saved budget`, await page.locator('#task-budget').inputValue(), '7');
      check(`${profile.name}: reload starts no fake provider`, (await calls()).length, beforeReload);
      if (!profile.hasTouch) {
        await page.keyboard.press('Escape');
        check(`${profile.name}: Escape closes detail panel`, await page.locator('#drawer').isHidden());
        check(`${profile.name}: Escape returns focus to its task card`, await page.evaluate(() => document.activeElement.dataset.card), created.id);
      } else {
        await page.getByRole('button', { name: 'Close panel', exact: true }).tap();
        check(`${profile.name}: touch closes panel`, await page.locator('#drawer').isHidden());
      }
      // With changes events suppressed, the board's real 2s refresh timer must
      // refresh both snapshot and selected detail without another click/reopen.
      const pollingTitle = `SYNTHETIC ${profile.name} board polling ${Date.now()}`;
      const pollingCreated = await api('/commands', { action: 'create', task: {
        title: pollingTitle, project_path: fixture.project, model: 'codex/fixture-model', effort: 'high',
        permission_mode: 'acceptEdits', execution_timeout_seconds: 3,
        isolation: { kind: 'worktree', base_branch: 'main' }, prompt: 'SYNTHETIC polling only. BUDGET_FIXTURE_CASE=stall',
      } });
      check(`${profile.name}: polling regression creates a real budgeted task`, pollingCreated.status, 200);
      const pollingTask = pollingCreated.value.tasks.find(value => value.title === pollingTitle);
      assert.ok(pollingTask);
      row.polling_task_id = pollingTask.id;
      let suppressedChangeStreams = 0;
      const suppressChanges = async route => { suppressedChangeStreams++; await route.fulfill({ status: 200,
        contentType: 'text/event-stream', body: ': SYNTHETIC transport suppression: board polling regression\n\n' }); };
      const pollingHandler = staleBoard(pollingTask.id, row.polling_board_responses, true);
      await page.route(changesPath, suppressChanges);
      await page.route(boardPath, pollingHandler);
      const beforePollingReads = browserSnapshotReads;
      expectedNavigation = true;
      await page.goto(`${fixture.base}/api/coordination/ui?budget_qa_poll=${encodeURIComponent(profile.name)}#task=${pollingTask.id}`);
      expectedNavigation = false;
      await page.locator('#task-budget').waitFor({ state: 'visible' });
      await page.waitForFunction(() => document.getElementById('task-budget')?.disabled);
      const pollingDraft = `SYNTHETIC unsent ${profile.name} draft during polling only.`;
      await focusDraft(page, pollingDraft);
      const interruptedPolling = await settled(pollingTask.id, 'interrupted');
      await interruptDetail(page, `${profile.name}: board-only polling`, pollingDraft);
      check(`${profile.name}: board-only timer fetched fresh task snapshots`, browserSnapshotReads > beforePollingReads + 1);
      check(`${profile.name}: board-only regression suppressed changes events`, suppressedChangeStreams, 1);
      check(`${profile.name}: board-only regression observed repeated board polls`, row.polling_board_responses.length >= 2);
      check(`${profile.name}: polling updated the already selected task`, await page.locator('#drawer-title').innerText(), pollingTitle);
      row.polling_snapshot_reads = browserSnapshotReads - beforePollingReads;
      await screenshot(page, profile, 'board-poll-interrupted');
      await page.unroute(boardPath, pollingHandler);
      await page.unroute(changesPath, suppressChanges);

      // Isolate live-stream reconnection from board timers. Production board
      // contents/revisions are retained; only polling is disabled by the fixture.
      const noPolling = async route => {
        const response = await route.fetch(); const value = await response.json(); value.refreshing = false;
        await route.fulfill({ response, contentType: 'application/json', body: JSON.stringify(value) });
      };
      await page.route(boardPath, noPolling);
      await activate(page.getByRole('button', { name: 'Close panel', exact: true }), profile);
      const initialLive = page.waitForResponse(response => response.url() === `${fixture.base}/api/coordination/changes`);
      await activate(page.locator('#refresh'), profile);
      check(`${profile.name}: live changes stream opens before owned restart`, (await initialLive).status(), 200);
      await pause(150);
      expectedOwnedRestart = true;
      const providerCallsBeforeRestart = (await calls()).length;
      row.owned_restart = await ownedRestart(profile);
      check(`${profile.name}: server restart has a distinct owned process`, row.owned_restart.prior_server_pid !== row.owned_restart.restarted_server_pid);
      check(`${profile.name}: owned restart starts no provider`, (await calls()).length, providerCallsBeforeRestart);
      row.notice_before_explicit_refresh = { status: await page.locator('#status').innerText(), error: await page.locator('#error').innerText() };
      const beforeReconnect = browserChangeRequests;
      const reconnected = page.waitForResponse(response => response.url() === `${fixture.base}/api/coordination/changes`);
      await activate(page.locator('#refresh'), profile);
      check(`${profile.name}: explicit Refresh reconnects changes stream after restart`, (await reconnected).status(), 200);
      check(`${profile.name}: explicit Refresh opens one new stream`, browserChangeRequests, beforeReconnect + 1);
      await pause(100);
      expectedOwnedRestart = false;
      row.notice_after_explicit_refresh = { status: await page.locator('#status').innerText(), error: await page.locator('#error').innerText() };
      check(`${profile.name}: successful Refresh clears prior connection error`, row.notice_after_explicit_refresh.error, '');
      await activate(page.locator(`[data-card="${pollingTask.id}"]`), profile);
      await waitSettledControls(page);
      const reconnectDraft = `SYNTHETIC unsent ${profile.name} draft after live reconnect.`;
      await focusDraft(page, reconnectDraft);
      const readsAfterRefresh = browserSnapshotReads;
      const sendAfterReconnect = await api('/commands', { action: 'send', id: pollingTask.id,
        prompt: 'SYNTHETIC explicit turn after host restart. BUDGET_FIXTURE_CASE=stall' });
      check(`${profile.name}: explicit synthetic turn starts after reconnect`, sendAfterReconnect.status, 200);
      await page.waitForFunction(() => document.getElementById('task-budget')?.disabled &&
        document.querySelector('#detail .status')?.textContent === 'Working');
      check(`${profile.name}: reconnected stream presents active state without another Refresh`, await page.locator('#task-budget').isDisabled());
      await screenshot(page, profile, 'reconnected-working');
      const expiredAfterReconnect = await settled(pollingTask.id, 'interrupted');
      await interruptDetail(page, `${profile.name}: reconnected live changes`, reconnectDraft);
      check(`${profile.name}: reconnected stream fetches subsequent snapshots`, browserSnapshotReads > readsAfterRefresh);
      check(`${profile.name}: restarted task retains its provider session`, expiredAfterReconnect.session_id, interruptedPolling.session_id);
      row.reconnect_snapshot_reads = browserSnapshotReads - readsAfterRefresh;
      row.explicit_refreshes_after_restart = 1;
      row.board_polling_disabled_during_reconnect = true;
      await screenshot(page, profile, 'reconnected-interrupted');
      await page.unroute(boardPath, noPolling);
      row.passed = true;
    } catch (error) {
      row.error = error.stack;
      try { await atomicWrite(join(output, `${profile.name}-failure.png`), await page.screenshot({ fullPage: true }));
        await atomicWrite(join(output, `${profile.name}-failure.txt`), await page.locator('body').innerText()); } catch {}
    } finally {
      report.profiles.push(row);
      await atomicWrite(join(output, 'browser-report.json'), JSON.stringify(report, null, 2) + '\n');
      expectedNavigation = true;
      await page.close();
    }
  }
  report.unexpected_resource_errors = report.resources.filter(resource => resource.status >= 400 && !resource.expected_owned_restart && !(resource.status === 404 && new URL(resource.url).pathname === '/favicon.ico'));
  report.unexpected_request_failures = report.request_failures.filter(request => !request.expected_owned_restart && !(request.error === 'net::ERR_ABORTED' && (request.expected_navigation || new URL(request.url).pathname === '/api/coordination/changes')));
  report.unexpected_console_errors = report.console.filter(message => !message.expected_owned_restart && !(message.text.includes('404') && message.location.url && new URL(message.location.url).pathname === '/favicon.ico'));
  check('No unexpected browser resource failures', report.unexpected_resource_errors, []);
  check('No unexpected browser request failures', report.unexpected_request_failures, []);
  check('No unexpected browser console errors', report.unexpected_console_errors, []);
  report.passed = report.profiles.every(row => row.passed) && report.page_errors.length === 0 && report.checks.every(row => row.passed);
} catch (error) {
  report.fatal_error = error.stack;
} finally {
  await browser.close();
  await atomicWrite(join(output, 'browser-report.json'), JSON.stringify(report, null, 2) + '\n');
}
console.log(JSON.stringify({ passed: report.passed, profiles: report.profiles, checks: report.checks.length, page_errors: report.page_errors }));
process.exitCode = report.passed ? 0 : 1;
