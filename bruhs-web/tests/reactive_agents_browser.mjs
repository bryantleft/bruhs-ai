// Run after the isolated UI build: node bruhs-web/tests/reactive_agents_browser.mjs.
// Fresh browser contexts and mocked APIs only; no user profile, providers or OS clipboard.
import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.env.BRUHS_REACTIVE_UI_DIST ?? path.join(root, 'target/reactive-ui-dist'));
const output = path.join(root, 'target/reactive-ui-qa');
const index = await readFile(path.join(dist, 'index.html'), 'utf8');
const assets = await readdir(dist);
assert.ok(assets.some(file => file.endsWith('_bg.wasm')), 'Build isolated reactive WASM assets first');
const sourceTime = Math.max(...await Promise.all([
  'bruhs-web/src/components/reactive_automations.rs',
  'bruhs-web/src/components/reactive_automations/setup.rs',
].map(async file => (await stat(path.join(root, file))).mtimeMs)));
assert.ok((await stat(path.join(dist, 'index.html'))).mtimeMs >= sourceTime, 'Refuse stale UI assets; rebuild the isolated dist');
assert.ok(index.includes('.wasm'), 'The fixture must serve the production Trunk bundle');
await mkdir(output, { recursive: true });

async function playwrightModule() {
  if (process.env.BRUHS_PLAYWRIGHT_MODULE) return import(pathToFileURL(process.env.BRUHS_PLAYWRIGHT_MODULE).href);
  try { return await import('playwright'); } catch { /* Reuse an existing local installation. */ }
  for (const entry of await readdir(path.join(homedir(), '.npm/_npx'))) {
    try { return await import(pathToFileURL(path.join(homedir(), '.npm/_npx', entry, 'node_modules/playwright/index.mjs')).href); }
    catch { /* Not every cached npx package contains Playwright. */ }
  }
  throw new Error('Install Playwright or set BRUHS_PLAYWRIGHT_MODULE.');
}

const { chromium } = await playwrightModule();
const chrome = process.env.BRUHS_CHROME ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined);
const server = createServer(async (req, res) => {
  const pathname = new URL(req.url, 'http://127.0.0.1').pathname;
  if (pathname === '/__reactive-fixture-stream') {
    res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Access-Control-Allow-Origin': '*', 'Cache-Control': 'no-store' });
    res.write(': isolated browser fixture\n\n');
    const keepAlive = setInterval(() => res.write(': connected\n\n'), 1000);
    req.on('close', () => clearInterval(keepAlive));
    return;
  }
  const file = path.resolve(dist, `.${pathname === '/' ? '/index.html' : decodeURIComponent(pathname)}`);
  if (!file.startsWith(`${dist}${path.sep}`)) { res.writeHead(403); res.end(); return; }
  try {
    const bytes = await readFile(file);
    res.writeHead(200, { 'Content-Type': { '.wasm': 'application/wasm', '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(file)] ?? 'application/octet-stream' });
    res.end(bytes);
  } catch { res.writeHead(404); res.end('Missing isolated browser fixture asset'); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, executablePath: chrome });
const passed = [];
const baselineConsole = [];
const check = (condition, label) => { assert.ok(condition, label); passed.push(label); if (process.env.BRUHS_BROWSER_DEBUG) console.log(label); };
const pause = millis => new Promise(resolve => setTimeout(resolve, millis));
const upsert = (records, record) => { const index = records.findIndex(value => value.id === record.id); if (index < 0) records.push(record); else records[index] = record; };
const empty = () => ({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
const assertKeys = (value, keys) => assert.deepEqual(Object.keys(value).sort(), keys.slice().sort());
const assertAgent = agent => assertKeys(agent, ['id', 'name', 'instructions', 'memory', 'model', 'project_path', 'enabled']);
const assertSource = source => assertKeys(source, ['id', 'name', 'enabled', 'kind']);
const assertSubscription = subscription => assertKeys(subscription, ['id', 'source_id', 'agent_id', 'instructions', 'enabled', 'auto_run', 'debounce_ms', 'throttle_ms']);

async function fixture(context) {
  const state = { snapshot: empty(), commands: [], delayMs: 180, unexpected: [], errors: [], failNext: false, expectedValidationErrors: 0, validationResponses: 0 };
  await context.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async text => { window.__reactiveCopied = text; } } });
  });
  await context.route('**/*', async route => {
    const request = route.request();
    const url = new URL(request.url());
    if (!url.pathname.startsWith('/api/')) {
      if (url.origin === origin) return route.continue();
      state.unexpected.push(request.url());
      return route.abort();
    }
    const json = async (value, status = 200) => route.fulfill({ status, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, POST, OPTIONS', 'Cache-Control': 'no-store' }, body: JSON.stringify(value) });
    if (request.method() === 'OPTIONS') return json({});
    if (url.pathname === '/api/reactive' && request.method() === 'GET') return json(state.snapshot);
    if (url.pathname === '/api/weekly-reports' && request.method() === 'GET') return json({ presets: [], reports: [], next_before: null });
    if (url.pathname === '/api/reactive/commands') {
      const command = request.postDataJSON();
      state.commands.push(command);
      if (command.action === 'apply' && command.change.type === 'save_automation') {
        assertKeys(command.change, ['type', 'automation']);
        const { agent, source, subscription } = command.change.automation;
        assertKeys(command.change.automation, ['agent', 'source', 'subscription']);
        assertAgent(agent); assertSource(source); assertSubscription(subscription);
        assert.equal(subscription.agent_id, agent.id);
        assert.equal(subscription.source_id, source.id);
        assert.ok(agent.enabled && source.enabled && subscription.enabled, 'The single reviewed bundle enables the complete automation');
      }
      await pause(state.delayMs);
      if (state.failNext) { state.failNext = false; state.validationResponses += 1; return json({ error: 'Choose an existing absolute working folder.' }, 400); }
      switch (command.action) {
        case 'apply': {
          assertKeys(command, ['action', 'change']);
          const change = command.change;
          if (change.type === 'save_automation') {
            const { agent, source, subscription } = structuredClone(change.automation);
            upsert(state.snapshot.agents, agent);
            upsert(state.snapshot.sources, source);
            upsert(state.snapshot.subscriptions, subscription);
          } else if (change.type === 'save_agent') {
            assertAgent(change.agent);
            upsert(state.snapshot.agents, structuredClone(change.agent));
          } else if (change.type === 'save_source') {
            assertSource(change.source);
            assertKeys(change.source.kind, ['type', 'path', 'pattern', 'recursive', 'settle_ms', 'poll_ms']);
            upsert(state.snapshot.sources, structuredClone(change.source));
          } else if (change.type === 'save_subscription') {
            assertSubscription(change.subscription);
            upsert(state.snapshot.subscriptions, structuredClone(change.subscription));
          } else assert.fail(`Unexpected direct change: ${change.type}`);
          break;
        }
        case 'set_automation_enabled': {
          assertKeys(command, ['action', 'id', 'enabled']);
          const subscription = state.snapshot.subscriptions.find(value => value.id === command.id);
          assert.ok(subscription, 'Pause and resume use the stable subscription identity');
          subscription.enabled = command.enabled;
          if (command.enabled) {
            state.snapshot.agents.find(value => value.id === subscription.agent_id).enabled = true;
            state.snapshot.sources.find(value => value.id === subscription.source_id).enabled = true;
          }
          break;
        }
        case 'decide_proposal': {
          assertKeys(command, ['action', 'id', 'approve']);
          const proposal = state.snapshot.proposals.find(value => value.id === command.id);
          assert.ok(proposal, 'Review must name the displayed proposal');
          proposal.status = command.approve ? 'approved' : 'rejected';
          if (command.approve && proposal.change.type === 'set_memory') {
            state.snapshot.agents.find(value => value.id === proposal.change.agent_id).memory = proposal.change.memory;
          } else if (command.approve && proposal.change.type === 'save_automation') {
            const { agent, source, subscription } = structuredClone(proposal.change.automation);
            upsert(state.snapshot.agents, agent);
            upsert(state.snapshot.sources, source);
            upsert(state.snapshot.subscriptions, subscription);
          }
          break;
        }
        case 'approve_run': case 'cancel_run': {
          assertKeys(command, ['action', 'id']);
          const run = state.snapshot.runs.find(value => value.id === command.id);
          assert.ok(run, 'Run action must name the displayed run');
          run.status = command.action === 'approve_run' ? 'queued' : 'cancelled';
          break;
        }
        default: assert.fail(`Unexpected reactive mutation: ${command.action}`);
      }
      return json(state.snapshot);
    }
    if (request.method() !== 'GET' || url.pathname === '/api/agent') {
      state.unexpected.push(`${request.method()} ${url.pathname}`);
      return json({ error: 'Live actions are forbidden in this browser fixture.' }, 500);
    }
    if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/__reactive-fixture-stream` });
    if (/\/(meetings|people|credentials|tasks|threads|skills|models)$/.test(url.pathname)) return json([]);
    return json({});
  });
  return state;
}

async function ready(page) {
  await page.waitForFunction(() => {
    const panel = document.querySelector('section[aria-labelledby="reactive-title"]');
    const fieldset = panel?.querySelector('fieldset');
    return fieldset && !fieldset.disabled;
  });
}
async function refresh(page) {
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.waitForFunction(() => {
    const panel = document.querySelector('section[aria-labelledby="reactive-title"]');
    const control = [...panel.querySelectorAll('button')].find(button => button.textContent.trim() === 'Refresh');
    return control && !control.disabled;
  });
}
async function closeEditors(page) {
  const close = page.getByRole('button', { name: 'Close', exact: true });
  while (await close.count()) await close.first().click();
}
function advancedPanel(page) { return page.locator('details').filter({ has: page.getByText('Advanced settings', { exact: true }) }).first(); }
function automationCard(page, name) { return page.locator('article').filter({ has: page.getByRole('heading', { name, exact: true }) }).first(); }
function agentCard(page) { return advancedPanel(page).locator('article').filter({ has: page.getByRole('heading', { name: 'Bill assistant', exact: true }) }).first(); }
function sourceCard(page) { return advancedPanel(page).locator('article').filter({ has: page.getByRole('heading', { name: 'Scans', exact: true }) }).first(); }
function runFixture(agentId, status, id = 'run-review') {
  return { id, agent_id: agentId, source_id: 'source-fixture', subscription_id: 'subscription-fixture', event_id: 'event-fixture', status, prompt: 'A scan arrived. Extract a payment draft for review.', thread_id: 'reactive-fixture-thread', session_id: null, created_at: 1000, updated_at: 1000, not_before: 1000, result: null, error: null };
}

async function primaryFlow(page, state, cell) {
  const noRecords = () => state.snapshot.agents.length + state.snapshot.sources.length + state.snapshot.subscriptions.length === 0;
  check(!await page.getByRole('button', { name: 'New agent', exact: true }).isVisible(), `${cell.name}: separate component setup is hidden under Advanced settings`);
  const newAutomation = page.getByRole('button', { name: 'New automation', exact: true });
  if (cell.hasTouch) await newAutomation.tap(); else await newAutomation.click();
  const purpose = 'Read incoming bills and prepare payment details as a draft. Flag unclear amounts and never submit payments.';
  const memory = 'Use household utilities for electricity bills. Preserve uncertainty and payment references.';
  await page.getByLabel('Automation name', { exact: true }).fill('Scan assistant');
  await page.getByLabel('What should this automation do?', { exact: true }).fill(purpose);
  await page.getByLabel('Automation working folder', { exact: true }).fill('/fixture/scans');
  await page.getByLabel('Model', { exact: true }).fill('claude-sonnet-4-6');
  await page.getByLabel('Folder to watch', { exact: true }).fill('/fixture/scans');
  await page.getByText('Advanced options', { exact: true }).click();
  await page.getByLabel('Automation memory', { exact: true }).fill(memory);
  check(await page.getByLabel('Run read-only drafts automatically', { exact: true }).isChecked(), `${cell.name}: the unified setup selects automatic read-only drafts explicitly`);
  check(state.commands.length === 0 && noRecords(), `${cell.name}: filling setup creates no partial agent, source, subscription or POST`);
  await page.getByRole('button', { name: 'Review automation', exact: true }).click();
  const review = page.locator('article').filter({ has: page.getByRole('heading', { name: 'Review automation', exact: true }) });
  await review.waitFor();
  const text = await review.textContent();
  check([purpose, memory, '/fixture/scans', '*.pdf', 'claude-sonnet-4-6', 'Read-only drafts. No direct writes or payments.', 'Runs read-only drafts automatically.'].every(value => text.includes(value)), `${cell.name}: review includes complete purpose, memory, source, model and access policy`);
  check(state.commands.length === 0 && noRecords(), `${cell.name}: Review automation is local and saves no partial configuration`);
  await page.getByRole('heading', { name: 'Review automation', exact: true }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(output, `${cell.name}-primary-review.png`), fullPage: false });
  await page.getByRole('button', { name: 'Back to edit', exact: true }).click();
  check(await page.getByLabel('Automation working folder', { exact: true }).inputValue() === '/fixture/scans', `${cell.name}: returning from review retains the complete setup`);
  await page.getByRole('button', { name: 'Review automation', exact: true }).click();
  state.failNext = true;
  await page.getByRole('button', { name: 'Enable automation', exact: true }).click();
  check(await page.getByRole('button', { name: 'Enable automation', exact: true }).isDisabled(), `${cell.name}: activation disables repeated submission while pending`);
  await ready(page);
  await page.getByRole('alert').filter({ hasText: 'Choose an existing absolute working folder.' }).waitFor();
  await page.waitForTimeout(2500);
  check(noRecords() && await review.isVisible(), `${cell.name}: failed activation leaves no partial configuration and retains the review`);
  check(await page.getByRole('alert').filter({ hasText: 'Choose an existing absolute working folder.' }).isVisible(), `${cell.name}: activation failure survives status polling`);
  const failed = structuredClone(state.commands.at(-1));
  const beforeEnable = state.commands.length;
  await page.getByRole('button', { name: 'Enable automation', exact: true }).click();
  await ready(page);
  await page.getByRole('heading', { name: 'Review automation', exact: true }).waitFor({ state: 'hidden' });
  check(state.commands.length === beforeEnable + 1 && state.commands.at(-1).change.type === 'save_automation', `${cell.name}: one Enable automation action submits exactly one atomic bundle`);
  check(JSON.stringify(state.commands.at(-1)) === JSON.stringify(failed), `${cell.name}: activation retry preserves reviewed values and all stable identities`);
  check(state.snapshot.agents.length === 1 && state.snapshot.sources.length === 1 && state.snapshot.subscriptions.length === 1 && state.snapshot.subscriptions[0].auto_run, `${cell.name}: successful activation creates all three enabled components with automatic drafts`);
  const subscriptionId = state.snapshot.subscriptions[0].id;
  let before = state.commands.length;
  await automationCard(page, 'Scan assistant').getByRole('button', { name: 'Pause automation', exact: true }).click();
  await ready(page);
  check(state.commands.length === before + 1 && JSON.stringify(state.commands.at(-1)) === JSON.stringify({ action: 'set_automation_enabled', id: subscriptionId, enabled: false }), `${cell.name}: Pause automation is one command against the subscription identity`);
  before = state.commands.length;
  await automationCard(page, 'Scan assistant').getByRole('button', { name: 'Resume automation', exact: true }).click();
  await ready(page);
  check(state.commands.length === before + 1 && state.snapshot.subscriptions[0].enabled && state.snapshot.subscriptions[0].auto_run, `${cell.name}: one Resume action preserves automatic-draft policy`);
  await automationCard(page, 'Scan assistant').getByRole('button', { name: 'Chat', exact: true }).click();
  check(await page.getByLabel('Message to agent', { exact: true }).isVisible(), `${cell.name}: chat is available directly from the automation card`);
  await closeEditors(page);
  check(await page.getByRole('heading', { name: 'Activity and conversations', exact: true }).isVisible(), `${cell.name}: activity remains outside Advanced settings`);
  await automationCard(page, 'Scan assistant').scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(output, `${cell.name}-primary-enabled.png`), fullPage: false });

  // Loading an existing command configuration must not leak its hidden working
  // folder into a later new automation that uses a different working folder.
  state.snapshot.sources[0].kind = { type: 'command', command: 'printf ready', cwd: '/fixture/old-project', poll_ms: 5000, timeout_ms: 99000 };
  await refresh(page);
  await ready(page);
  await automationCard(page, 'Scan assistant').getByRole('button', { name: 'Edit automation', exact: true }).click();
  await page.getByText('Advanced options', { exact: true }).click();
  check(await page.getByLabel('Command working folder, if different', { exact: true }).inputValue() === '/fixture/old-project', `${cell.name}: editing an existing command exposes its exact working folder`);
  await page.getByRole('button', { name: 'Cancel setup', exact: true }).click();
  await page.getByRole('button', { name: 'New automation', exact: true }).click();
  await page.getByLabel('Automation name', { exact: true }).fill('New command assistant');
  await page.getByLabel('What should this automation do?', { exact: true }).fill('Prepare a status draft.');
  await page.getByLabel('Automation working folder', { exact: true }).fill('/fixture/new-project');
  await page.getByLabel('Automation source type', { exact: true }).selectOption('command');
  await page.getByLabel('Command to run', { exact: true }).fill('printf ready');
  await page.getByText('Advanced options', { exact: true }).click();
  check(await page.getByLabel('Command working folder, if different', { exact: true }).inputValue() === '' && await page.getByLabel('Command timeout in seconds', { exact: true }).inputValue() === '30', `${cell.name}: new command setup clears another automation\'s hidden folder and timeout`);
  await page.getByRole('button', { name: 'Review automation', exact: true }).click();
  check((await review.textContent()).includes('/fixture/new-project') && !(await review.textContent()).includes('/fixture/old-project'), `${cell.name}: command review uses the newly chosen working folder`);
  await page.getByRole('button', { name: 'Cancel setup', exact: true }).click();

  state.snapshot = empty();
  await refresh(page);
  await ready(page);
}

async function bundledProposalFlow(page, state, cell) {
  for (const enabled of [true, false]) {
    const prefix = enabled ? 'proposed' : 'paused-proposed';
    const automation = {
      agent: { id: `${prefix}-agent`, name: `${prefix} meeting assistant`, instructions: 'Read completed summaries and prepare follow-up task drafts with uncertainty.', memory: 'Keep the complete meeting reference. Never send a message.', model: 'claude-sonnet-4-6', project_path: '/fixture/meeting-notes', enabled },
      source: { id: `${prefix}-source`, name: `${prefix} summaries`, enabled, kind: { type: 'app_event', topic: 'meeting.summary' } },
      subscription: { id: `${prefix}-subscription`, source_id: `${prefix}-source`, agent_id: `${prefix}-agent`, instructions: 'Prepare a reviewed task draft from each completed summary.', enabled, auto_run: enabled, debounce_ms: 0, throttle_ms: 0 },
    };
    state.snapshot.proposals = [{ id: `${prefix}-proposal`, change: { type: 'save_automation', automation }, status: 'pending', created_at: 2000 }];
    const previousCount = state.snapshot.agents.length + state.snapshot.sources.length + state.snapshot.subscriptions.length;
    const before = state.commands.length;
    await refresh(page);
    const review = page.locator('article').filter({ has: page.getByRole('heading', { name: 'Setup proposal awaiting review', exact: true }) });
    await review.waitFor();
    const text = await review.textContent();
    check([automation.agent.instructions, automation.agent.memory, automation.agent.model, automation.agent.project_path, 'meeting.summary', 'Read-only drafts. No direct writes or payments.', 'Agent state', 'Source state', 'Automation state', 'Event policy'].every(value => text.includes(value)), `${cell.name}: ${prefix} bundled proposal has complete readable purpose, source, memory, model, access and states`);
    check(await review.locator('pre:visible').count() === 0 && await review.getByText('Advanced setup definition', { exact: true }).isVisible(), `${cell.name}: ${prefix} proposal review is readable without opening raw configuration`);
    check(state.commands.length === before && state.snapshot.agents.length + state.snapshot.sources.length + state.snapshot.subscriptions.length === previousCount, `${cell.name}: ${prefix} proposal stays inactive until owner approval`);
    const button = enabled ? 'Enable automation' : 'Save automation';
    if (!enabled) {
      const sourceState = review.locator('dt').filter({ hasText: /^Source state$/ }).locator('..').locator('dd');
      check((await sourceState.textContent()).trim() === 'Paused', `${cell.name}: paused bundled proposal exposes its inactive source state`);
    }
    await review.getByRole('button', { name: button, exact: true }).click();
    await ready(page);
    check(state.commands.length === before + 1 && JSON.stringify(state.commands.at(-1)) === JSON.stringify({ action: 'decide_proposal', id: `${prefix}-proposal`, approve: true }), `${cell.name}: ${prefix} proposal approval is one decision against the exact reviewed identity`);
    check(state.snapshot.agents.find(value => value.id === automation.agent.id).enabled === enabled && state.snapshot.sources.find(value => value.id === automation.source.id).enabled === enabled && state.snapshot.subscriptions.find(value => value.id === automation.subscription.id).enabled === enabled && state.snapshot.subscriptions.find(value => value.id === automation.subscription.id).auto_run === enabled, `${cell.name}: ${prefix} proposal preserves every reviewed enabled flag and execution policy`);
  }
}

try {
  for (const cell of [{ name: 'desktop', width: 1280, height: 900 }, { name: 'compact', width: 640, height: 480 }, { name: 'touch', width: 390, height: 844, isMobile: true, hasTouch: true }]) {
    const context = await browser.newContext({ viewport: { width: cell.width, height: cell.height }, isMobile: !!cell.isMobile, hasTouch: !!cell.hasTouch });
    const state = await fixture(context);
    const page = await context.newPage();
    page.on('pageerror', error => state.errors.push(error.message));
    page.on('console', message => {
      if (message.type() !== 'error') return;
      const text = message.text();
      // Chromium already reports these four production index CSP sources as
      // invalid. Keep them in the report instead of attributing them to this UI.
      if (/^The source list for the Content Security Policy directive '(img-src|media-src)' contains an invalid source: 'https?:\/\/\[::1\]:\*'\. It will be ignored\.$/.test(text)) {
        baselineConsole.push(text);
      } else if (state.expectedValidationErrors < state.validationResponses && text === 'Failed to load resource: the server responded with a status of 400 (Bad Request)') {
        state.expectedValidationErrors += 1;
      } else state.errors.push(text);
    });
    if (process.env.BRUHS_BROWSER_DEBUG) page.on('console', message => { if (message.type() === 'error') console.error(cell.name, message.text()); });
    await page.goto(`${origin}/#/automations`);
    await page.getByRole('heading', { name: 'Automations', exact: true }).waitFor();
    await ready(page);
    if (cell.hasTouch && await page.getByRole('complementary', { name: 'Sidebar navigation', exact: true }).count()) {
      await page.locator('div[aria-hidden="true"].fixed.inset-0').tap({ position: { x: cell.width - 12, y: 60 } });
      await page.getByRole('complementary', { name: 'Sidebar navigation', exact: true }).waitFor({ state: 'hidden' });
      check(true, `${cell.name}: sidebar drawer dismisses with a touch on its scrim`);
    }
    check(await page.getByText('Drafts only', { exact: true }).isVisible(), `${cell.name}: read-only scope is visible`);
    await page.screenshot({ path: path.join(output, `${cell.name}-empty.png`), fullPage: false });
    const cdp = await context.newCDPSession(page);
    await cdp.send('Network.enable');
    await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 400, downloadThroughput: 500_000 / 8, uploadThroughput: 500_000 / 8 });

    await primaryFlow(page, state, cell);
    await page.getByText('Advanced settings', { exact: true }).click();

    const newAgent = page.getByRole('button', { name: 'New agent', exact: true });
    if (cell.hasTouch) await newAgent.tap(); else await newAgent.click();
    await page.getByLabel('Agent name', { exact: true }).fill('Bill assistant');
    await page.getByLabel('Working folder', { exact: true }).fill('/fixture/scans');
    await page.getByLabel('Purpose and instructions', { exact: true }).fill('Read scans and prepare drafts. Never make payments.');
    await page.getByRole('button', { name: 'Save agent', exact: true }).click();
    check(await page.getByRole('button', { name: 'Save agent', exact: true }).isDisabled(), `${cell.name}: pending save disables repeated activation`);
    await ready(page);
    check(state.snapshot.agents.length === 1 && !state.snapshot.agents[0].enabled, `${cell.name}: new agent is paused by default`);
    await closeEditors(page);
    await agentCard(page).getByRole('button', { name: 'Edit and memory', exact: true }).click();
    await page.getByLabel('Persistent memory', { exact: true }).fill('Electricity bills belong to household utilities. Keep uncertain references for review.');
    state.failNext = true;
    await page.getByRole('button', { name: 'Save agent', exact: true }).click();
    await ready(page);
    await page.getByRole('alert').filter({ hasText: 'Choose an existing absolute working folder.' }).waitFor();
    // Cross one automatic status-refresh cycle: polling must not erase the
    // failed mutation's explanation or the owner's unsaved replacement memory.
    await page.waitForTimeout(2500);
    check(await page.getByRole('alert').filter({ hasText: 'Choose an existing absolute working folder.' }).isVisible(), `${cell.name}: polling retains the failed save explanation`);
    check((await page.getByLabel('Persistent memory', { exact: true }).inputValue()).includes('household utilities'), `${cell.name}: server validation preserves editor fields`);
    await page.getByRole('button', { name: 'Save agent', exact: true }).click();
    await ready(page);
    check(state.snapshot.agents[0].memory.includes('household utilities'), `${cell.name}: persistent memory saves through the typed command`);
    await closeEditors(page);
    await agentCard(page).getByRole('button', { name: 'Enable', exact: true }).click();
    await ready(page);

    await page.getByRole('button', { name: 'Watch scans', exact: true }).click();
    await page.getByLabel('Folder to watch', { exact: true }).fill('/fixture/scans');
    await page.getByRole('button', { name: 'Save source', exact: true }).click();
    await ready(page);
    check(state.snapshot.sources[0].kind.type === 'folder' && state.snapshot.sources[0].kind.pattern === '*.pdf' && !state.snapshot.sources[0].enabled, `${cell.name}: guided scanner source retains safe defaults`);
    await closeEditors(page);
    await sourceCard(page).getByRole('button', { name: 'Enable', exact: true }).click();
    await ready(page);
    await page.getByRole('button', { name: 'Connect source to agent', exact: true }).click();
    await page.getByLabel('Subscription agent', { exact: true }).selectOption(state.snapshot.agents[0].id);
    await page.getByLabel('Subscription source', { exact: true }).selectOption(state.snapshot.sources[0].id);
    await page.getByText('Subscription enabled', { exact: true }).click();
    await page.getByRole('button', { name: 'Save subscription', exact: true }).click();
    await ready(page);
    check(state.snapshot.subscriptions.length === 1 && state.snapshot.subscriptions[0].enabled && !state.snapshot.subscriptions[0].auto_run, `${cell.name}: subscription keeps review before execution`);
    await closeEditors(page);
    await page.getByText('Advanced settings', { exact: true }).click();
    const legacyId = state.snapshot.subscriptions[0].id;
    let beforeLegacyToggle = state.commands.length;
    await automationCard(page, 'Bill assistant').getByRole('button', { name: 'Pause automation', exact: true }).click();
    await ready(page);
    check(state.commands.length === beforeLegacyToggle + 1 && state.commands.at(-1).id === legacyId && !state.snapshot.subscriptions[0].auto_run, `${cell.name}: legacy automation pauses with one command without changing event-review policy`);
    beforeLegacyToggle = state.commands.length;
    await automationCard(page, 'Bill assistant').getByRole('button', { name: 'Resume automation', exact: true }).click();
    await ready(page);
    check(state.commands.length === beforeLegacyToggle + 1 && !state.snapshot.subscriptions[0].auto_run, `${cell.name}: legacy automation resumes without silently enabling automatic drafts`);
    state.snapshot.sources[0].kind.settle_ms = 0;
    await refresh(page);
    const beforeLegacyEdit = state.commands.length;
    await automationCard(page, 'Bill assistant').getByRole('button', { name: 'Edit automation', exact: true }).click();
    await page.getByText('Advanced options', { exact: true }).click();
    check(!await page.getByLabel('Run read-only drafts automatically', { exact: true }).isChecked(), `${cell.name}: editing a legacy automation preserves per-event review`);
    await page.getByRole('button', { name: 'Review automation', exact: true }).click();
    check(await page.getByText('Each event waits for review before a read-only draft runs. Writes still use workflow review.', { exact: true }).isVisible(), `${cell.name}: legacy review clearly states its existing execution policy`);
    check((await page.locator('article').filter({ has: page.getByRole('heading', { name: 'Review automation', exact: true }) }).textContent()).includes('File settle time: 0 seconds'), `${cell.name}: valid zero-settle source settings survive a legacy edit and review`);
    await page.getByRole('button', { name: 'Cancel setup', exact: true }).click();
    check(state.commands.length === beforeLegacyEdit, `${cell.name}: inspecting legacy settings does not re-save or enable anything`);

    state.snapshot.proposals = [{ id: 'proposal-memory', change: { type: 'set_memory', agent_id: state.snapshot.agents[0].id, memory: 'Full replacement memory: keep payee, due date, reference and uncertainty.' }, status: 'pending', created_at: 1000 }];
    state.snapshot.runs = [runFixture(state.snapshot.agents[0].id, 'awaiting_review')];
    await refresh(page);
    await page.getByRole('heading', { name: 'Setup proposal awaiting review', exact: true }).waitFor();
    check(await page.getByText('Full replacement memory: keep payee, due date, reference and uncertainty.', { exact: false }).count() > 0, `${cell.name}: complete proposed memory is visible before approval`);
    await page.getByRole('button', { name: 'Approve setup change', exact: true }).click();
    await ready(page);
    check(state.snapshot.proposals[0].status === 'approved', `${cell.name}: setup approval names the reviewed proposal`);
    await page.getByRole('button', { name: 'Run read-only draft', exact: true }).click();
    await ready(page);
    check(state.snapshot.runs[0].status === 'queued', `${cell.name}: event approval queues only the named read-only run`);
    await page.getByRole('button', { name: 'Cancel run', exact: true }).click();
    await ready(page);
    check(state.snapshot.runs[0].status === 'cancelled', `${cell.name}: queued event can be cancelled`);

    const result = JSON.stringify({ summary: 'Prepare a task to review the electricity bill', operations: [{ type: 'create_task', title: 'Review the scanned electricity bill' }] });
    state.snapshot.runs = [{ ...runFixture(state.snapshot.agents[0].id, 'completed', 'run-draft'), result }];
    await refresh(page);
    await page.getByRole('button', { name: 'Copy draft', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Copy draft', exact: true }).click();
    check(await page.evaluate(() => window.__reactiveCopied) === result, `${cell.name}: copy draft retains the exact proposal without touching the OS clipboard`);
    const countBeforePreview = state.commands.length;
    await page.getByRole('button', { name: 'Preview changes', exact: true }).click();
    await page.getByRole('heading', { name: 'Preview changes', exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Start transaction', exact: true }).isDisabled(), `${cell.name}: imported draft waits for independent Write grants`);
    check(state.commands.length === countBeforePreview, `${cell.name}: preview never executes a reactive or provider action`);
    check(await page.getByRole('button', { name: 'Toggle Tasks Write access', exact: true }).getAttribute('aria-pressed') === 'false', `${cell.name}: preview does not grant write permissions`);
    await bundledProposalFlow(page, state, cell);
    await page.getByRole('heading', { name: 'Automations', exact: true }).scrollIntoViewIfNeeded();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth + 1);
    check(!overflow, `${cell.name}: viewport has no horizontal overflow`);
    await page.screenshot({ path: path.join(output, `${cell.name}-configured.png`), fullPage: false });
    check(state.unexpected.length === 0, `${cell.name}: no live model, external service or other action was requested`);
    check(state.errors.length === 0, `${cell.name}: no browser console errors or uncaught exceptions (${state.errors.join('; ')})`);
    await context.close();
  }
  console.log(JSON.stringify({ passed: passed.length, checks: passed, screenshots: output, baselineConsoleWarnings: [...new Set(baselineConsole)], throttling: 'Slow 3G applied after initial bundle load; interactions verified under throttling.' }, null, 2));
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
