// Pause in the app-wide workflow notice, on the real WASM bundle and the
// browser's IndexedDB, through headless Chrome and CDP (cdp_harness.mjs).
// Synthetic local records only; no request reaches a server.
//
// A focus workflow (task, calendar block, note: three steps) runs in the
// background. Every read-write transaction's completion is delayed four
// seconds (the commit has happened; the app learns of it later), so each
// step takes a while and the driver, which waits for the workflow store to
// settle between steps, is caught mid-step. Pause is pressed on Today's
// notice once by pointer and once by keyboard (Tab from the control before
// it, then Enter). Each time the job must pause after its current step, the
// notice must read "Workflow paused, saved steps are retained", a reload
// must keep it paused, and Resume on Agent workflows must continue it; the
// last Resume completes it with each record made once.
//
//   (cd bruhs-web && trunk build --dist ../target/web-dist)
//   node bruhs-web/tests/workflow_pause_browser.mjs [dist]
// BRUHS_QA_DIR sets the output directory; BRUHS_QA_PORT and
// BRUHS_QA_CHROME_PORT the static server's and Chrome's ports.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertFreshDist, checklist, launchChrome, newContext, newPage, serveDist, sleep } from './cdp_harness.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.argv[2] ?? process.env.BRUHS_WEB_DIST ?? path.join(root, 'target/web-dist'));
const startedAt = new Date().toISOString();
const output = path.resolve(process.env.BRUHS_QA_DIR ?? path.join(root, 'target/qa/workflow-pause', startedAt.replace(/[:.]/g, '-')));
const port = Number(process.env.BRUHS_QA_PORT ?? 39352);
const chromePort = Number(process.env.BRUHS_QA_CHROME_PORT ?? 9352);
const DELAY_MS = 4000;
const sources = [
  'bruhs-web/src/components/workflow_status.rs', 'bruhs-web/src/workflow_executor.rs',
  'bruhs-web/src/components/workflow_history.rs', 'bruhs-web/src/components/automations_page.rs',
  'bruhs-web/src/storage/workflow.rs', 'bruhs-web/src/durable_metadata.rs',
  'bruhs-models/src/logic/workflow_review.rs', 'bruhs-models/src/models/automation.rs',
];
const freshness = await assertFreshDist(root, dist, sources);
await mkdir(output, { recursive: true });
const git = args => { try { return execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim(); } catch { return null; } };
const report = {
  harness: 'workflow_pause_browser.mjs', startedAt, dist, output, commitDelayMs: DELAY_MS,
  revision: git(['rev-parse', 'HEAD']), dirtyFiles: git(['status', '--porcelain'])?.split('\n').filter(Boolean) ?? null,
  indexSha256: createHash('sha256').update(await readFile(path.join(dist, 'index.html'))).digest('hex'), freshness,
  viewport: '1280x800', evidence: {}, checks: [],
};

const PAUSED = 'Workflow paused, saved steps are retained';
const BUTTON = text => `[...document.querySelectorAll('button')].find(b => b.textContent.trim() === ${JSON.stringify(text)} && b.offsetParent !== null)`;
const PAUSE = `document.querySelector('[aria-label="Pause background workflows"]')`;
const NOTICE = `(document.querySelector('[aria-label="Workflow status"] p')?.textContent ?? '').trim()`;

const server = await serveDist(dist, port);
const chrome = await launchChrome({ port: chromePort, profile: path.join(root, 'target/qa/chrome-workflow-pause') });
report.chrome = chrome.version;
const check = checklist('pause');
const evidence = report.evidence;

async function automation(page) {
  const rows = await page.readStore('metadata');
  const row = rows.find(entry => entry.key === 'bruhs.automationState');
  let value = row?.value ?? null;
  if (typeof value === 'string') value = JSON.parse(value);
  return value;
}
const job = async page => (await automation(page))?.jobs?.[0] ?? null;
async function waitForJob(page, accept, timeout) {
  const deadline = Date.now() + timeout;
  let last = null;
  while (Date.now() < deadline) {
    last = await job(page);
    if (last && accept(last)) return last;
    await sleep(150);
  }
  return last;
}
const settled = page => page.waitFor(`!document.querySelector('[aria-label="Thread list loading status"]')`, 15_000);
const summary = state => state && { status: state.status, next_step: state.next_step, auto_run: state.auto_run };

async function made(page) {
  const [tasks, events, captures] = await Promise.all(['work_items', 'events', 'captures'].map(store => page.readStore(store)));
  return {
    tasks: tasks.filter(task => task.title === 'Finish the highest-impact task').length,
    events: events.filter(event => event.title === 'Protected focus').length,
    captures: captures.filter(capture => JSON.stringify(capture).includes('Protected 45 minutes for the highest-impact task.')).length,
  };
}

async function toToday(page) {
  await page.evaluate(`location.hash = '#/today'`);
  return page.waitFor(`!!document.querySelector('#today-title')`, 20_000);
}

async function toWorkflows(page) {
  await page.evaluate(`location.hash = '#/automations'`);
  return page.waitFor(`!!document.querySelector('#automations-title') && !document.body.innerText.includes('Loading local workflow records')`, 20_000);
}

// Waits until step `step` (1-based) has committed in storage while the app
// still reports one fewer: the step is in flight, and Pause lands mid-step.
async function midStep(page, step) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    const [durable, notice, pauseReady] = await Promise.all([job(page), page.evaluate(NOTICE), page.evaluate(`!!${PAUSE} && !${PAUSE}.disabled`)]);
    const shown = Number(notice.match(/(\d+) steps? saved/)?.[1] ?? -1);
    if (durable?.next_step === step && shown === step - 1 && pauseReady) return { durable: summary(durable), notice };
    await sleep(50);
  }
  return null;
}

// After Pause: the step in flight commits, then the job pauses; nothing
// further runs for two more step lengths.
async function pausedAfter(page, step, how) {
  const disabled = await page.evaluate(`${PAUSE}?.disabled ?? null`);
  check(disabled === true, `${how}: Pause is disabled once pressed`, disabled);
  const paused = await waitForJob(page, state => state.status === 'paused', 4 * DELAY_MS);
  check(paused?.status === 'paused' && paused.next_step === step, `${how}: the job pauses after its current step (step ${step})`, summary(paused));
  await sleep(2 * DELAY_MS + 1000);
  const later = await job(page);
  check(later?.status === 'paused' && later.next_step === step, `${how}: no later step runs`, summary(later));
  const notice = await page.evaluate(NOTICE);
  check(notice === PAUSED, `${how}: the notice reads "${PAUSED}"`, notice);
  check(!(await page.evaluate(`!!${PAUSE}`)), `${how}: Pause is gone once the job is paused`);
  return { paused: summary(paused), later: summary(later), notice, focusAfter: await page.focused() };
}

async function reloadKeepsPaused(page, step, how) {
  await page.reload();
  const shown = await page.waitFor(`${NOTICE} === ${JSON.stringify(PAUSED)}`, 60_000);
  check(shown, `${how}: after a reload the notice still reads paused`, await page.evaluate(NOTICE).catch(() => null));
  await sleep(2 * DELAY_MS + 1000);
  const state = await job(page);
  check(state?.status === 'paused' && state.next_step === step, `${how}: a reload keeps the job paused at step ${step}`, summary(state));
  await settled(page);
}

async function resume(page, how) {
  check(await toWorkflows(page), `${how}: Agent workflows opens`);
  const ready = await page.waitFor(`(() => { const b = document.querySelector('[data-testid="resume-workflow"]'); return !!b && !b.disabled && !b.closest('fieldset:disabled'); })()`, 30_000);
  check(ready, `${how}: Resume is offered`);
  const clicked = await page.click(`document.querySelector('[data-testid="resume-workflow"]')`);
  check(clicked.clicked, `${how}: Resume is pressed`, clicked);
  const running = await waitForJob(page, state => state.status === 'running', 4 * DELAY_MS);
  check(running?.status === 'running' && running.auto_run === true, `${how}: Resume continues in the background`, summary(running));
}

const context = await newContext(chrome.browser);
const page = await newPage(chrome.browser, server.origin, { context, width: 1280, height: 800 });
try {
  await page.goto(`${server.origin}/#/automations`);
  check(await page.waitFor(`!!document.querySelector('#automations-title')`, 60_000) && await settled(page), 'Agent workflows opens');
  // Write grants for the focus workflow's three domains.
  for (const domain of ['Tasks', 'Calendar', 'Capture']) {
    const toggle = `document.querySelector('[aria-label="Toggle ${domain} Write access"]')`;
    await page.waitFor(`!!${toggle} && !${toggle}.disabled && !${toggle}.closest('fieldset:disabled')`, 30_000);
    if (await page.evaluate(`${toggle}.getAttribute('aria-pressed')`) !== 'true') await page.click(toggle);
    check(await page.waitFor(`${toggle}.getAttribute('aria-pressed') === 'true' && !${toggle}.closest('fieldset:disabled')`, 10_000), `${domain} Write is granted`);
  }
  check((await page.click(BUTTON('New dry run'))).clicked, 'New dry run is pressed');
  check(await page.waitFor(`!!${BUTTON('Start transaction')} && !${BUTTON('Start transaction')}.closest('fieldset:disabled')`, 15_000), 'the dry run offers Start transaction');
  check((await page.click(BUTTON('Start transaction'))).clicked, 'Start transaction is pressed');
  const background = `document.querySelector('[data-testid="enable-workflow-background"]')`;
  check(await page.waitFor(`!!${background} && !${background}.disabled && !${background}.closest('fieldset:disabled')`, 15_000), 'Run in background is offered');
  // From here every read-write commit reaches the app four seconds late.
  await page.qa({ commitDelayMs: DELAY_MS });
  check((await page.click(background)).clicked, 'Run in background is pressed');
  const started = await waitForJob(page, state => state.auto_run === true && state.status === 'running', 4 * DELAY_MS);
  check(started?.auto_run === true, 'the job runs in the background', summary(started));
  check(await toToday(page), 'Today opens while it runs');

  // Pointer: Pause while step 1 is in flight.
  const pointerAt = await midStep(page, 1);
  evidence.pointerPressedAt = pointerAt;
  check(!!pointerAt, 'pointer: the notice shows Background workflow running with Pause while step 1 commits', pointerAt);
  evidence.runningNotice = await page.evaluate(NOTICE);
  await page.screenshot(path.join(output, 'running.png'));
  const pressed = await page.click(PAUSE);
  check(pressed.clicked, 'pointer: Pause is pressed where the notice shows it', pressed);
  evidence.pointer = await pausedAfter(page, 1, 'pointer');
  check(JSON.stringify(await made(page)) === JSON.stringify({ tasks: 1, events: 0, captures: 0 }), 'pointer: only step 1 made its record', await made(page));
  await page.screenshot(path.join(output, 'paused-pointer.png'));
  await reloadKeepsPaused(page, 1, 'pointer');
  await resume(page, 'pointer');

  // Keyboard: Tab from the control before Pause, then Enter, while step 2
  // is in flight.
  check(await toToday(page), 'Today opens again');
  const keyboardAt = await midStep(page, 2);
  evidence.keyboardPressedAt = keyboardAt;
  check(!!keyboardAt, 'keyboard: Pause is offered while step 2 commits', keyboardAt);
  await page.evaluate(`(() => { const before = [...document.querySelectorAll('nav[aria-label="Workspace"] button, nav[aria-label="Workspace"] a')].filter(el => el.offsetParent !== null).at(-1); before.focus(); })()`);
  evidence.keyboardStart = await page.focused();
  await page.press('Tab');
  const onPause = await page.evaluate(`document.activeElement === ${PAUSE}`);
  evidence.keyboardFocus = await page.focused();
  check(onPause, 'keyboard: Tab from the control before it reaches Pause', evidence.keyboardFocus);
  check(evidence.keyboardFocus.focusVisible === true, 'keyboard: Pause shows its focus', evidence.keyboardFocus);
  await page.press('Enter');
  evidence.keyboard = await pausedAfter(page, 2, 'keyboard');
  check(evidence.keyboard.focusAfter.text === 'Open workflows', 'keyboard: Pause hands focus to Open workflows, which stays, as on desktop', evidence.keyboard.focusAfter);
  check(JSON.stringify(await made(page)) === JSON.stringify({ tasks: 1, events: 1, captures: 0 }), 'keyboard: steps 1 and 2 made one record each', await made(page));
  await page.screenshot(path.join(output, 'paused-keyboard.png'));
  await reloadKeepsPaused(page, 2, 'keyboard');
  await resume(page, 'keyboard');

  // The last Resume completes the workflow with each record made once.
  const done = await waitForJob(page, state => state.status === 'completed', 6 * DELAY_MS);
  check(done?.status === 'completed' && done.next_step === 3, 'after the last Resume the job completes all three steps', summary(done));
  check(JSON.stringify(await made(page)) === JSON.stringify({ tasks: 1, events: 1, captures: 1 }), 'each record is made once', await made(page));
  check(await toToday(page), 'Today opens at the end');
  check(await page.waitFor(`${NOTICE} === 'Workflow completed, saved locally'`, 4 * DELAY_MS), 'the notice reads Workflow completed, saved locally', await page.evaluate(NOTICE));
  await page.screenshot(path.join(output, 'completed.png'));
  evidence.delayedCommits = await page.evaluate('window.__qa.delayedCommits');
} catch (error) {
  check(false, 'the run finished without an exception', String(error.stack ?? error));
} finally {
  check(page.unexpected.length === 0, 'no request left the machine', page.unexpected);
  check(page.runtimeErrors.length === 0, 'no uncaught page error', page.runtimeErrors);
  evidence.refusedApiRequests = page.refusedApi.length;
  evidence.consoleErrors = page.consoleErrors.slice(0, 20);
  report.finishedAt = new Date().toISOString();
  report.checks = check.all;
  report.summary = check.summary();
  await writeFile(path.join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  await page.close();
  await context.dispose();
  await chrome.close();
  await server.close();
  console.log(JSON.stringify({ ...report.summary, output }));
  process.exitCode = report.summary.failed === 0 ? 0 : 1;
}
