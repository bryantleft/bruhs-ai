// TODAY-04 and TODAY-06 on the real WASM bundle and the browser's IndexedDB,
// through headless Chrome and CDP (cdp_harness.mjs). Synthetic local
// records only; no request reaches a server. The desktop counterparts are
// `a_replayed_apply_or_a_taken_block_id_adds_nothing` and
// `undo_refuses_a_changed_or_adopted_focus_block`
// (bruhs-desktop/src/views/workspace/tests/today_acceptance.rs).
//
// The clock is fixed at 08:00 in Los Angeles on 2026-11-13, before the
// 09:00 to 18:00 workday, so a plan's block ids (`plan-<now>-<task>`) are the
// same in every tab that reviews the same tasks. Tabs share one IndexedDB.
//
//   TODAY-04  A second tab that reviewed the same plan applies it after the
//             first tab did; a stale tab reviews and applies it after the
//             first tab reloaded; a plan whose block id is already a
//             calendar record is applied. Each is refused with the shared
//             message, and no block, link or receipt is added or replaced.
//   TODAY-06  A focus block renamed in the web event dialog makes Undo
//             refuse; renamed back, Undo works. A block another writer links
//             to another task makes Undo refuse; with that link gone and the
//             other block deleted in the dialog, Undo removes the rest.
//
//   (cd bruhs-web && trunk build --dist ../target/web-dist)
//   node bruhs-web/tests/today_replay_browser.mjs [dist]
// BRUHS_QA_DIR sets the output directory; BRUHS_QA_PORT and
// BRUHS_QA_CHROME_PORT the static server's and Chrome's ports.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertFreshDist, checklist, launchChrome, newContext, newPage, serveDist, sleep } from './cdp_harness.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.argv[2] ?? process.env.BRUHS_WEB_DIST ?? path.join(root, 'target/web-dist'));
const startedAt = new Date().toISOString();
const output = path.resolve(process.env.BRUHS_QA_DIR ?? path.join(root, 'target/qa/today-replay', startedAt.replace(/[:.]/g, '-')));
const port = Number(process.env.BRUHS_QA_PORT ?? 39351);
const chromePort = Number(process.env.BRUHS_QA_CHROME_PORT ?? 9351);
const sources = [
  'bruhs-web/src/components/today_page.rs', 'bruhs-web/src/task_saves/planning.rs',
  'bruhs-web/src/storage/day_plans.rs', 'bruhs-web/src/components/event_create_dialog.rs',
  'bruhs-web/src/components/timegrid_view.rs', 'bruhs-models/src/models/planning/day_plan.rs',
  'bruhs-models/src/models/planning.rs',
];
const freshness = await assertFreshDist(root, dist, sources);
await mkdir(output, { recursive: true });

// The refusals' wording is shared with desktop; read it from the source.
const dayPlan = await readFile(path.join(root, 'bruhs-models/src/models/planning/day_plan.rs'), 'utf8');
const message = variant => {
  const match = dayPlan.match(new RegExp(`Self::${variant} => \\{?\\s*"([^"]+)"`));
  assert(match, `bruhs-models has no message for ${variant}`);
  return match[1];
};
const ALREADY_APPLIED = message('AlreadyApplied');
const EVENT_CHANGED = message('GeneratedEventChanged');
const LINK_CHANGED = message('GeneratedLinkChanged');

const NOW = Date.parse('2026-11-13T16:00:00Z');
const TIMEZONE = 'America/Los_Angeles';
const CLOCK = `(() => {
  globalThis.__qaNow = ${NOW};
  const NativeDate = Date;
  globalThis.Date = new Proxy(NativeDate, {
    construct(target, args) { return Reflect.construct(target, args.length ? args : [globalThis.__qaNow]); },
    apply() { return new NativeDate(globalThis.__qaNow).toString(); },
    get(target, key) { return key === 'now' ? () => globalThis.__qaNow : Reflect.get(target, key); },
  });
})();`;
const block = taskId => `plan-${NOW}-${taskId}`;

const git = args => { try { return execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim(); } catch { return null; } };
const report = {
  harness: 'today_replay_browser.mjs', startedAt, dist, output, clock: new Date(NOW).toISOString(), timezone: TIMEZONE,
  revision: git(['rev-parse', 'HEAD']), dirtyFiles: git(['status', '--porcelain'])?.split('\n').filter(Boolean) ?? null,
  indexSha256: createHash('sha256').update(await readFile(path.join(dist, 'index.html'))).digest('hex'), freshness,
  viewport: '1280x800', messages: { ALREADY_APPLIED, EVENT_CHANGED, LINK_CHANGED }, scenarios: {}, checks: [],
};

const BUTTON = text => `[...document.querySelectorAll('button')].find(b => b.textContent.trim() === ${JSON.stringify(text)} && b.offsetParent !== null)`;
const IN_DIALOG = (label, text) => `[...document.querySelectorAll(${JSON.stringify(`[role=dialog][aria-label="${label}"] button`)})].find(b => b.textContent.trim() === ${JSON.stringify(text)})`;
const APPLIED = `[...document.querySelectorAll('[role=status]')].some(s => s.textContent.includes('Day plan applied.'))`;

function task(id, title, status = 'Todo') {
  return { id, project_id: null, title, description: null, status, priority: 'High', due_date: null, planning: { cycle: null, dependency_ids: [], estimate_minutes: 45, recurrence: null, time_block_event_ids: [], waiting_on: null }, created_at: NOW - 3_600_000, updated_at: NOW - 3_600_000 };
}

const server = await serveDist(dist, port);
const chrome = await launchChrome({ port: chromePort, profile: path.join(root, 'target/qa/chrome-today-replay') });
report.chrome = chrome.version;
const check = checklist('setup');

// Today is up and the thread list's loading strip has gone (it moves the
// page as it goes, so a pointer would miss).
async function todayReady(page) {
  const ready = await page.waitFor(`!!document.querySelector('#today-title') && !!${BUTTON('Plan my day')}`, 60_000);
  return ready && page.waitFor(`!document.querySelector('[aria-label="Thread list loading status"]')`, 15_000);
}

async function openToday(page) {
  await page.goto(`${page.origin}/#/today`);
  return todayReady(page);
}

function writeRecords(page, puts) {
  return page.evaluate(`new Promise((resolve, reject) => {
    const puts = ${JSON.stringify(puts)};
    const open = indexedDB.open('bruhs');
    open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('the app has not created its database')); };
    open.onerror = () => reject(new Error(String(open.error)));
    open.onsuccess = () => {
      const db = open.result;
      const tx = db.transaction(Object.keys(puts), 'readwrite');
      for (const [store, records] of Object.entries(puts)) for (const record of records) tx.objectStore(store).put(record);
      tx.oncomplete = () => { db.close(); resolve(true); };
      tx.onabort = () => { db.close(); reject(new Error(String(tx.error))); };
    };
  })`);
}

async function planState(page) {
  const [tasks, events, metadata] = await Promise.all(['work_items', 'events', 'metadata'].map(store => page.readStore(store)));
  let receipt = metadata.find(row => row.key === 'bruhs.last-day-plan')?.value ?? null;
  if (typeof receipt === 'string') receipt = JSON.parse(receipt);
  return {
    links: Object.fromEntries(tasks.map(item => [item.id, item.planning.time_block_event_ids]).sort(([left], [right]) => left.localeCompare(right))),
    events: events.map(event => ({ id: event.id, title: event.title, start: event.start_at_ms, end: event.end_at_ms })).sort((left, right) => left.id.localeCompare(right.id)),
    receipt: receipt && { id: receipt.id, events: receipt.ownership.events.map(event => event.id), links: receipt.ownership.links },
  };
}
const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);

async function waitForState(page, accept, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    last = await planState(page);
    if (accept(last)) return last;
    await sleep(100);
  }
  return last;
}

async function review(page, blocks) {
  assert((await page.click(BUTTON('Plan my day'))).clicked, 'Plan my day is clickable');
  const shown = await page.waitFor(`!!${IN_DIALOG('Review day plan', 'Apply plan')}`, 10_000);
  const text = shown ? await page.evaluate(`document.querySelector('[role=dialog][aria-label="Review day plan"]').innerText`) : '';
  check(shown && text.includes(`${blocks} focus block(s)`), `the review shows ${blocks} focus block(s)`, text.slice(0, 200));
}

async function shownMessage(page, expected) {
  const found = await page.waitFor(`document.body.innerText.includes(${JSON.stringify(expected)})`, 10_000);
  const where = await page.evaluate(`(() => { const hits = [...document.querySelectorAll('[role=alert]')].filter(el => el.textContent.includes(${JSON.stringify(expected)})); return hits.map(el => el.closest('[role=dialog]') ? 'review dialog' : el.closest('[data-banner], [aria-live]') && !el.matches('[role=alert]') ? 'banner' : 'page'); })()`);
  return { found, where };
}

async function dismissErrors(page) {
  for (let attempt = 0; attempt < 4; attempt++) {
    const dismissed = await page.evaluate(`(() => { const button = [...document.querySelectorAll('[role=alert] button, [role=alert] ~ button')].find(b => b.textContent.trim() === 'Dismiss'); if (!button) return false; button.click(); return true; })()`);
    if (!dismissed) return;
    await sleep(150);
  }
}

// Renames a calendar block through the web event dialog and saves it.
async function rename(page, eventId, title) {
  await page.evaluate(`location.hash = '#/calendar'`);
  const blockSelector = `document.querySelector(${JSON.stringify(`[data-event-id="${eventId}"]`)})`;
  assert(await page.waitFor(`!!${blockSelector}`, 20_000), `the calendar shows ${eventId}`);
  assert((await page.click(blockSelector)).clicked, 'the block is clickable');
  assert(await page.waitFor(`!!document.querySelector('[role=dialog][aria-label="Calendar event"] input')`, 10_000), 'the event dialog opens');
  const heading = await page.evaluate(`document.querySelector('[role=dialog][aria-label="Calendar event"] h2')?.textContent.trim()`);
  await page.evaluate(`(() => { const input = document.querySelector('[role=dialog][aria-label="Calendar event"] input'); input.focus(); input.select(); })()`);
  await page.type(title);
  assert((await page.click(IN_DIALOG('Calendar event', 'Save'))).clicked, 'Save is clickable');
  await page.waitFor(`!document.querySelector('[role=dialog][aria-label="Calendar event"]')`, 10_000);
  return heading;
}

async function deleteInDialog(page, eventId) {
  await page.evaluate(`location.hash = '#/calendar'`);
  const blockSelector = `document.querySelector(${JSON.stringify(`[data-event-id="${eventId}"]`)})`;
  assert(await page.waitFor(`!!${blockSelector}`, 20_000), `the calendar shows ${eventId}`);
  assert((await page.click(blockSelector)).clicked, 'the block is clickable');
  assert(await page.waitFor(`!!${IN_DIALOG('Calendar event', 'Delete')}`, 10_000), 'the event dialog offers Delete');
  assert((await page.click(IN_DIALOG('Calendar event', 'Delete'))).clicked, 'Delete is clickable');
  // A confirmation, if the dialog asks for one.
  await sleep(300);
  const confirm = `[...document.querySelectorAll('[role=dialog] button')].find(b => /^(Delete|Delete event|Confirm)$/.test(b.textContent.trim()) && b.offsetParent !== null)`;
  if (await page.evaluate(`!!document.querySelector('[role=dialog]') && !!${confirm}`)) await page.click(confirm);
  await page.waitFor(`!document.querySelector('[role=dialog]')`, 10_000);
}

async function toToday(page) {
  await page.evaluate(`location.hash = '#/today'`);
  return page.waitFor(`!!document.querySelector('#today-title')`, 20_000);
}

async function seeded(context, tasks) {
  const page = await newPage(chrome.browser, server.origin, { context, width: 1280, height: 800, timezone: TIMEZONE, initScripts: [CLOCK] });
  check(await openToday(page), 'Today opens');
  await writeRecords(page, { work_items: tasks });
  await page.reload();
  check(await todayReady(page) && await page.waitFor(`document.body.innerText.includes(${JSON.stringify(tasks[0].title)})`, 20_000), 'the seeded tasks are listed');
  return page;
}

const scenario = async (name, body) => {
  check.scenario = name;
  const context = await newContext(chrome.browser);
  const pages = [];
  const evidence = report.scenarios[name] = { screenshots: [] };
  const shot = async (page, label) => { evidence.screenshots.push(await page.screenshot(path.join(output, `${name}-${label}.png`))); };
  try {
    await body({ context, pages, evidence, shot });
  } catch (error) {
    check(false, 'scenario finished without an exception', String(error.stack ?? error));
  } finally {
    for (const page of pages) {
      check(page.unexpected.length === 0, 'no request left the machine', page.unexpected);
      check(page.runtimeErrors.length === 0, 'no uncaught page error', page.runtimeErrors);
      evidence.consoleErrors = [...(evidence.consoleErrors ?? []), ...page.consoleErrors.slice(0, 10)];
      await page.close();
    }
    await context.dispose();
  }
};

try {
  await scenario('TODAY-04', async ({ context, pages, evidence, shot }) => {
    const first = await seeded(context, [task('task-focus', 'Protect focus')]);
    pages.push(first);
    // Two more tabs on the same records before anything is applied.
    const second = await newPage(chrome.browser, server.origin, { context, width: 1280, height: 800, timezone: TIMEZONE, initScripts: [CLOCK] });
    const stale = await newPage(chrome.browser, server.origin, { context, width: 1280, height: 800, timezone: TIMEZONE, initScripts: [CLOCK] });
    pages.push(second, stale);
    check(await openToday(second) && await openToday(stale), 'two more tabs open Today');
    await review(first, 1);
    await review(second, 1);

    // The first tab applies; its save is slowed so a second Apply can be tried.
    await first.qa({ commitDelayMs: 1500 });
    assert((await first.click(IN_DIALOG('Review day plan', 'Apply plan'))).clicked, 'Apply plan is clickable');
    const saving = await first.waitFor(`document.body.innerText.includes('Saving day plan…')`, 5_000);
    const busy = await first.evaluate(`(() => { const plan = ${BUTTON('Plan my day')}; return { planDisabled: !!plan && plan.disabled, reviewOpen: !!document.querySelector('[role=dialog][aria-label="Review day plan"]') }; })()`);
    evidence.whileSaving = { saving, ...busy };
    check(saving && busy.planDisabled && !busy.reviewOpen, 'while the first Apply saves, the review is closed and Plan my day waits', evidence.whileSaving);
    const applied = await waitForState(first, state => state.receipt && state.events.length === 1);
    await first.qa({ commitDelayMs: 0 });
    check(same(applied.events.map(event => [event.id, event.title]), [[block('task-focus'), 'Focus: Protect focus']]) && same(applied.links['task-focus'], [block('task-focus')]) && applied.receipt?.events.length === 1, 'the first Apply adds one block, one link and a receipt', applied);
    check(await first.waitFor(APPLIED, 10_000), 'the first tab shows Day plan applied.');
    evidence.applied = applied;

    // Replay: the second tab applies the same reviewed plan.
    assert((await second.click(IN_DIALOG('Review day plan', 'Apply plan'))).clicked, 'the second tab can press Apply plan');
    const replay = await shownMessage(second, ALREADY_APPLIED);
    evidence.replay = replay;
    check(replay.found, `a replayed Apply is refused with "${ALREADY_APPLIED}"`, replay);
    await sleep(500);
    check(same(await planState(second), applied), 'the replay adds no block, link or receipt', await planState(second));
    check(!(await second.evaluate(APPLIED)), 'the refused tab does not claim the plan as applied');
    await shot(second, 'replay-refused');

    // After the first tab reloads, a stale tab reviews the same plan and applies it.
    await first.reload();
    check(await todayReady(first) && await first.waitFor(APPLIED, 20_000), 'after a reload the first tab still shows the plan and its Undo');
    check(await first.evaluate(`!!${BUTTON('Undo')}`), 'Undo is offered after the reload');
    await review(stale, 1);
    assert((await stale.click(IN_DIALOG('Review day plan', 'Apply plan'))).clicked, 'the stale tab can press Apply plan');
    const afterReload = await shownMessage(stale, ALREADY_APPLIED);
    evidence.afterReload = afterReload;
    check(afterReload.found, `a replay after a reload is refused with "${ALREADY_APPLIED}"`, afterReload);
    await sleep(500);
    check(same(await planState(stale), applied), 'the replay after a reload adds nothing', await planState(stale));
    await shot(stale, 'replay-after-reload-refused');

    // A plan whose block id is already a calendar record.
    const taken = { id: block('task-brief'), title: 'Independently owned record', start_at_ms: NOW - 3_600_000, end_at_ms: NOW - 1_800_000, all_day: false, color: 'blueberry', notes: 'Synthetic record that holds a planned block id', created_at: NOW - 7_200_000, updated_at: NOW - 7_200_000, source: 'local', google_event_id: null, calendar_id: null, attendees: [], organizer: null, location: null, conferencing: null, recurring_event_id: null, html_link: null };
    await writeRecords(first, { work_items: [task('task-brief', 'Write the brief')], events: [taken] });
    await first.reload();
    check(await todayReady(first) && await first.waitFor(`document.body.innerText.includes('Write the brief')`, 20_000), 'the new task is listed');
    const before = await planState(first);
    await review(first, 1);
    const reviewed = await first.evaluate(`document.querySelector('[role=dialog][aria-label="Review day plan"]').innerText`);
    check(reviewed.includes('Write the brief') && !reviewed.includes('Protect focus'), 'only the new task is planned', reviewed.slice(0, 200));
    assert((await first.click(IN_DIALOG('Review day plan', 'Apply plan'))).clicked, 'Apply plan is clickable');
    const collision = await shownMessage(first, ALREADY_APPLIED);
    evidence.collision = collision;
    check(collision.found, `a plan whose block id is taken is refused with "${ALREADY_APPLIED}"`, collision);
    await sleep(500);
    const afterCollision = await planState(first);
    check(same(afterCollision, before), 'nothing is added: no block, link or receipt', afterCollision);
    check(afterCollision.events.find(event => event.id === block('task-brief'))?.title === 'Independently owned record', 'the taken record is not overwritten');
    await shot(first, 'taken-id-refused');

    // Once more after a reload: every tab sees one block for the first plan.
    await first.reload();
    check(await todayReady(first), 'Today opens after the last reload');
    const final = await planState(first);
    evidence.final = final;
    check(final.events.filter(event => event.title === 'Focus: Protect focus').length === 1 && same(final.links, { 'task-brief': [], 'task-focus': [block('task-focus')] }) && final.receipt?.id === applied.receipt.id, 'storage keeps one block, one link and the first receipt', final);
  });

  await scenario('TODAY-06-renamed', async ({ context, pages, evidence, shot }) => {
    const page = await seeded(context, [task('task-focus', 'Protect focus'), task('task-brief', 'Write the brief'), task('task-done', 'Already done', 'Done')]);
    pages.push(page);
    await review(page, 2);
    assert((await page.click(IN_DIALOG('Review day plan', 'Apply plan'))).clicked, 'Apply plan is clickable');
    const applied = await waitForState(page, state => state.receipt && state.events.length === 2);
    check(applied.events.length === 2 && applied.receipt?.events.length === 2, 'the plan adds two blocks and a receipt', applied);
    check(await page.waitFor(APPLIED, 10_000), 'Today shows Day plan applied.');

    // Renamed in the event dialog: Undo refuses and changes nothing.
    evidence.dialogHeading = await rename(page, block('task-focus'), 'Focus: Protect focus, moved');
    check(evidence.dialogHeading === 'Edit event', 'the block opens in the web event dialog for editing', evidence.dialogHeading);
    const edited = await waitForState(page, state => state.events.some(event => event.title === 'Focus: Protect focus, moved'));
    check(edited.events.some(event => event.title === 'Focus: Protect focus, moved'), 'the rename is saved');
    check(await toToday(page), 'back on Today');
    assert((await page.click(BUTTON('Undo'))).clicked, 'Undo is clickable');
    const refused = await shownMessage(page, EVENT_CHANGED);
    evidence.refused = refused;
    check(refused.found, `Undo refuses with "${EVENT_CHANGED}"`, refused);
    await sleep(500);
    check(same(await planState(page), edited), 'Undo changed nothing', await planState(page));
    check(await page.waitFor(`${APPLIED} && !!${BUTTON('Undo')}`, 5_000), 'Undo stays offered');
    await shot(page, 'undo-refused');
    await dismissErrors(page);

    // Renamed back: Undo works.
    await rename(page, block('task-focus'), 'Focus: Protect focus');
    const restored = await waitForState(page, state => state.events.some(event => event.title === 'Focus: Protect focus'));
    check(same(restored, applied), 'renamed back, the records read as planned', restored);
    check(await toToday(page), 'back on Today again');
    assert((await page.click(BUTTON('Undo'))).clicked, 'Undo is clickable again');
    const undone = await waitForState(page, state => !state.receipt && state.events.length === 0);
    evidence.undone = undone;
    check(undone.events.length === 0 && Object.values(undone.links).every(ids => ids.length === 0) && !undone.receipt, 'Undo removes both blocks, both links and the receipt', undone);
    check(await page.waitFor(`!${APPLIED}`, 5_000), 'Day plan applied. goes away');
    check(!(await page.evaluate(`document.body.innerText.includes(${JSON.stringify(EVENT_CHANGED)})`)), 'no refusal is left on screen');
    await shot(page, 'undone');
  });

  await scenario('TODAY-06-adopted', async ({ context, pages, evidence, shot }) => {
    const page = await seeded(context, [task('task-focus', 'Protect focus'), task('task-brief', 'Write the brief'), task('task-done', 'Already done', 'Done')]);
    pages.push(page);
    await review(page, 2);
    assert((await page.click(IN_DIALOG('Review day plan', 'Apply plan'))).clicked, 'Apply plan is clickable');
    const applied = await waitForState(page, state => state.receipt && state.events.length === 2);
    check(applied.events.length === 2, 'the plan adds two blocks', applied);

    // Another writer (another tab, an agent) links the focus block to the done task.
    const writer = await newPage(chrome.browser, server.origin, { context, width: 800, height: 600 });
    pages.push(writer);
    await writer.goto(`${server.origin}/__qa_blank`);
    await writer.waitFor(`document.readyState === 'complete'`);
    const done = (await writer.readStore('work_items')).find(item => item.id === 'task-done');
    await writeRecords(writer, { work_items: [{ ...done, planning: { ...done.planning, time_block_event_ids: [block('task-focus')] }, updated_at: NOW }] });
    const adopted = await planState(page);
    check(same(adopted.links['task-done'], [block('task-focus')]), 'the other writer linked the block to another task', adopted.links);
    assert((await page.click(BUTTON('Undo'))).clicked, 'Undo is clickable');
    const refused = await shownMessage(page, LINK_CHANGED);
    evidence.refused = refused;
    check(refused.found, `Undo refuses with "${LINK_CHANGED}"`, refused);
    await sleep(500);
    check(same(await planState(page), adopted), 'Undo changed nothing', await planState(page));
    check(await page.waitFor(`${APPLIED} && !!${BUTTON('Undo')}`, 5_000), 'Undo stays offered');
    await shot(page, 'undo-refused');
    await dismissErrors(page);

    // The other link goes; the user deletes the brief block in the dialog.
    await writeRecords(writer, { work_items: [{ ...done, updated_at: NOW }] });
    await page.reload();
    check(await todayReady(page) && await page.waitFor(APPLIED, 20_000), 'after a reload the plan and its Undo are back');
    await deleteInDialog(page, block('task-brief'));
    const deleted = await waitForState(page, state => !state.events.some(event => event.id === block('task-brief')));
    check(!deleted.events.some(event => event.id === block('task-brief')), 'the brief block is deleted');
    check(await toToday(page), 'back on Today');
    assert((await page.click(BUTTON('Undo'))).clicked, 'Undo is clickable again');
    const undone = await waitForState(page, state => !state.receipt);
    evidence.undone = undone;
    check(undone.events.length === 0 && Object.values(undone.links).every(ids => ids.length === 0) && !undone.receipt, 'Undo removes the other block and both links, and recreates nothing', undone);
    await shot(page, 'undone');
  });
} finally {
  report.finishedAt = new Date().toISOString();
  report.checks = check.all;
  report.summary = check.summary();
  await writeFile(path.join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  await chrome.close();
  await server.close();
  console.log(JSON.stringify({ ...report.summary, output }));
  process.exitCode = report.summary.failed === 0 ? 0 : 1;
}
