// People profile checks on the real WASM bundle and the browser's IndexedDB,
// through headless Chrome and CDP (cdp_harness.mjs). People reads and the
// rename and tracking changes are answered by in-process fixtures for two
// synthetic contacts; any other write is refused and recorded, and nothing
// leaves the machine.
//
//   Follow ups  A profile lists only its own person's follow ups (the
//               replies owed in their conversations), with no saved
//               relationship details, as `bruhs_models::person_follow_ups`
//               rules (the web fix from ce0e78b8). One opens its
//               conversation, and Back to People returns to the profile.
//   PEOPLE-09   Rename and the opt-out toggle each offer Undo; Undo sends
//               the earlier value back and the profile comes back intact,
//               also after a reload.
//
//   (cd bruhs-web && trunk build --dist ../target/web-dist)
//   node bruhs-web/tests/people_profile_browser.mjs [dist]
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
const output = path.resolve(process.env.BRUHS_QA_DIR ?? path.join(root, 'target/qa/people-profile', startedAt.replace(/[:.]/g, '-')));
const port = Number(process.env.BRUHS_QA_PORT ?? 39354);
const chromePort = Number(process.env.BRUHS_QA_CHROME_PORT ?? 9354);
const sources = [
  'bruhs-web/src/components/people_page.rs', 'bruhs-web/src/components/people_page/profile.rs',
  'bruhs-web/src/components/people_page/relationship.rs', 'bruhs-web/src/components/people_page/back.rs',
  'bruhs-web/src/workflow_executor/people_actions.rs', 'bruhs-web/src/components/inbox_page.rs',
  'bruhs-models/src/models/relationship.rs', 'bruhs-models/src/models/people_changes.rs',
];
const freshness = await assertFreshDist(root, dist, sources);
await mkdir(output, { recursive: true });
const git = args => { try { return execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim(); } catch { return null; } };
const report = {
  harness: 'people_profile_browser.mjs', startedAt, dist, output,
  revision: git(['rev-parse', 'HEAD']), dirtyFiles: git(['status', '--porcelain'])?.split('\n').filter(Boolean) ?? null,
  indexSha256: createHash('sha256').update(await readFile(path.join(dist, 'index.html'))).digest('hex'), freshness,
  viewport: '1280x800', evidence: {}, checks: [],
};

const T0 = 1_700_000_000_000;
const person = (id, display_name) => ({ id, display_name, aliases: [], voice_embedding: null, observations: [], tracked: true, created_at: T0, updated_at: T0 });
const data = { people: new Map([['ada', person('ada', 'Ada Synthetic')], ['bea', person('bea', 'Bea Other')]]), renames: [], tracked: [], blocked: [] };
const original = structuredClone(data.people.get('ada'));
function api({ method, url, body }) {
  const at = url.pathname;
  if (at.endsWith('/stream')) return 'stream';
  if (method === 'POST' && at === '/api/people/profiles') {
    const ids = JSON.parse(body ?? '{}').ids ?? [];
    return { body: ids.filter(id => data.people.has(id)).map(id => data.people.get(id)) };
  }
  if (method === 'POST' && at === '/api/people/resolve-merges') return { body: [] };
  const rename = at.match(/^\/api\/people\/([^/]+)\/rename$/);
  if (method === 'PATCH' && rename) {
    const id = decodeURIComponent(rename[1]);
    const { display_name } = JSON.parse(body ?? '{}');
    data.renames.push({ id, display_name });
    data.people.set(id, { ...data.people.get(id), display_name, updated_at: data.people.get(id).updated_at + 1 });
    return { body: data.people.get(id) };
  }
  const tracked = at.match(/^\/api\/people\/([^/]+)\/tracked$/);
  if (method === 'PATCH' && tracked) {
    const id = decodeURIComponent(tracked[1]);
    const change = JSON.parse(body ?? '{}');
    data.tracked.push({ id, tracked: change.tracked });
    data.people.set(id, { ...data.people.get(id), tracked: change.tracked, updated_at: data.people.get(id).updated_at + 1 });
    return { body: data.people.get(id) };
  }
  if (!['GET', 'HEAD'].includes(method)) {
    data.blocked.push(`${method} ${at}`);
    return { status: 500, body: { error: 'People profile verification rejects unexpected writes.' } };
  }
  if (at === '/api/people/versions') return { body: { entries: [...data.people.values()].map(({ id, updated_at }) => ({ id, updated_at })).sort((left, right) => left.id.localeCompare(right.id)), next_after: null } };
  if (at === '/api/people') return { body: [...data.people.values()] };
  if (at === '/api/meetings') return { body: [] };
  if (at === '/api/reactive') return { body: { agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] } };
  if (at === '/api/weekly-reports') return { body: { presets: [], reports: [], next_before: null } };
  if (at === '/api/calendar/google/state') return { body: { connected: false, email: null, calendars: [], last_synced_at: null, last_error: null } };
  if (/\/(credentials|tasks|threads|skills|models|calendars)$/.test(at)) return { body: [] };
  return { body: {} };
}

const BUTTON = text => `[...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label') ?? b.textContent.trim()) === ${JSON.stringify(text)} && b.offsetParent !== null)`;
const CARD = id => `document.querySelector(${JSON.stringify(`#person-record-${id}`)})?.closest('li')`;
const settled = page => page.waitFor(`!document.querySelector('[aria-label="Thread list loading status"]')`, 15_000);

const server = await serveDist(dist, port);
const chrome = await launchChrome({ port: chromePort, profile: path.join(root, 'target/qa/chrome-people-profile') });
report.chrome = chrome.version;
const check = checklist('people');
const evidence = report.evidence;

async function importConversation(page, { sender, subject, message }) {
  assert((await page.click(BUTTON('Import conversation'))).clicked, 'Import conversation is clickable');
  assert(await page.waitFor(`document.activeElement === document.querySelector('[role=dialog] input[aria-label=Sender]')`), 'the import form focuses Sender');
  await page.type(sender);
  await page.evaluate(`document.querySelector('[role=dialog] input[aria-label=Subject]').focus()`);
  await page.type(subject);
  await page.evaluate(`document.querySelector('[role=dialog] textarea[aria-label=Message]').focus()`);
  await page.type(message);
  assert((await page.click(BUTTON('Import'))).clicked, 'Import is clickable');
  assert(await page.waitFor(`location.hash.startsWith('#/inbox/') && !!document.querySelector('[data-inbox-detail]')`), 'the import opens its conversation');
  const id = (await page.evaluate('location.hash')).slice('#/inbox/'.length);
  assert((await page.click(BUTTON('Back to Inbox'))).clicked, 'Back to Inbox is clickable');
  await page.waitFor(`!!document.querySelector('[aria-label="Search inbox"]')`);
  return id;
}

async function openPeople(page) {
  await page.evaluate(`location.hash = '#/people'`);
  return page.waitFor(`!!document.querySelector('#person-record-ada') && !!document.querySelector('#person-record-bea')`, 30_000);
}

async function followUps(page, id) {
  const card = CARD(id);
  if (await page.evaluate(`${card}?.querySelector('#person-record-${id}')?.getAttribute('aria-expanded')`) !== 'true') {
    await page.click(`document.querySelector('#person-record-${id}')`);
  }
  await page.waitFor(`${card}?.textContent.includes('Suggested follow ups') || ${card}?.textContent.includes('Private context')`, 10_000);
  await sleep(300);
  return page.evaluate(`(() => { const card = ${card}; const heading = [...card.querySelectorAll('p')].find(p => p.textContent.trim() === 'Suggested follow ups'); if (!heading) return []; return [...heading.parentElement.querySelectorAll('button')].map(b => b.textContent.trim()); })()`);
}

const context = await newContext(chrome.browser);
const page = await newPage(chrome.browser, server.origin, { context, width: 1280, height: 800, api });
try {
  await page.goto(`${server.origin}/#/people`);
  check(await page.waitFor(`!!document.querySelector('#person-record-ada')`, 60_000) && await settled(page), 'People lists the two contacts');
  await page.evaluate(`location.hash = '#/inbox'`);
  await page.waitFor(`!!${BUTTON('Import conversation')}`, 20_000);
  const adaConversation = await importConversation(page, { sender: 'Ada Synthetic', subject: 'Launch question', message: 'Can you send the deck?' });
  const beaConversation = await importConversation(page, { sender: 'Bea Other', subject: 'Budget question', message: 'Any update on the budget?' });
  const stored = await page.readStore('inbox_conversations');
  const linked = Object.fromEntries(stored.map(record => [record.id, record.participants[0]?.person_id ?? null]));
  evidence.linked = linked;
  check(linked[adaConversation] === 'ada' && linked[beaConversation] === 'bea', 'each import links its sender to their profile', linked);
  const relationships = await page.readStore('relationships');
  check(relationships.length === 0, 'neither contact has saved relationship details', relationships.length);

  check(await openPeople(page), 'People opens again');
  const adaFollowUps = await followUps(page, 'ada');
  evidence.adaFollowUps = adaFollowUps;
  check(JSON.stringify(adaFollowUps) === JSON.stringify(['Reply to Launch question']), "Ada's profile lists only the reply owed in her conversation", adaFollowUps);
  const beaFollowUps = await followUps(page, 'bea');
  evidence.beaFollowUps = beaFollowUps;
  check(JSON.stringify(beaFollowUps) === JSON.stringify(['Reply to Budget question']), "Bea's profile lists only hers", beaFollowUps);
  await page.screenshot(path.join(output, 'follow-ups.png'));

  // A follow up opens its conversation; Back to People returns to the profile.
  // One profile is open at a time, so open Ada's again.
  await followUps(page, 'ada');
  const opened = await page.click(`[...${CARD('ada')}.querySelectorAll('button')].find(b => b.textContent.trim() === 'Reply to Launch question')`);
  check(opened.clicked, 'the follow up is clickable', opened);
  check(await page.waitFor(`location.hash === ${JSON.stringify(`#/inbox/${adaConversation}`)} && !!${BUTTON('Back to People')}`, 10_000), 'it opens the conversation with Back to People');
  assert((await page.click(BUTTON('Back to People'))).clicked, 'Back to People is clickable');
  check(await page.waitFor(`location.hash === '#/people' && document.querySelector('#person-record-ada')?.getAttribute('aria-expanded') === 'true'`, 10_000), 'Back to People returns to the open profile');

  // PEOPLE-09: rename, then Undo rename.
  assert((await page.click(BUTTON('Rename Ada Synthetic'))).clicked, 'Rename is clickable');
  assert(await page.waitFor(`!!document.querySelector('input[aria-label="Profile name"]')`, 5_000), 'the name field shows');
  await page.evaluate(`(() => { const input = document.querySelector('input[aria-label="Profile name"]'); input.focus(); input.select(); })()`);
  await page.type('Ada Renamed');
  await page.press('Enter');
  check(await page.waitFor(`document.querySelector('#person-record-ada')?.textContent.includes('Ada Renamed')`, 10_000), 'the rename shows');
  const renameUndo = await page.waitFor(`!!document.querySelector('[data-people-undo]') && !!${BUTTON('Undo rename')}`, 10_000);
  evidence.renameSummary = await page.evaluate(`document.querySelector('[data-people-undo] [role=status]')?.textContent.trim() ?? null`);
  check(renameUndo, 'the rename offers Undo rename', evidence.renameSummary);
  await page.screenshot(path.join(output, 'rename-undo-offered.png'));
  assert((await page.click(BUTTON('Undo rename'))).clicked, 'Undo rename is clickable');
  check(await page.waitFor(`document.querySelector('#person-record-ada')?.textContent.includes('Ada Synthetic') && !document.querySelector('[data-people-undo]')`, 10_000), 'Undo rename brings the name back and clears its offer');
  check(JSON.stringify(data.renames) === JSON.stringify([{ id: 'ada', display_name: 'Ada Renamed' }, { id: 'ada', display_name: 'Ada Synthetic' }]), 'the rename and its undo each sent one request', data.renames);

  // PEOPLE-09: the opt-out toggle, then Undo tracking change.
  assert((await page.click(BUTTON('Untrack Ada Synthetic'))).clicked, 'Untrack is clickable');
  check(await page.waitFor(`${CARD('ada')}?.textContent.includes('Not tracked')`, 10_000), 'the opt-out shows Not tracked');
  const trackedUndo = await page.waitFor(`!!${BUTTON('Undo tracking change')}`, 10_000);
  evidence.trackedSummary = await page.evaluate(`document.querySelector('[data-people-undo] [role=status]')?.textContent.trim() ?? null`);
  check(trackedUndo, 'the opt-out offers Undo tracking change', evidence.trackedSummary);
  assert((await page.click(BUTTON('Undo tracking change'))).clicked, 'Undo tracking change is clickable');
  check(await page.waitFor(`!${CARD('ada')}?.textContent.includes('Not tracked') && !document.querySelector('[data-people-undo]') && !!${BUTTON('Untrack Ada Synthetic')}`, 10_000), 'Undo tracking change tracks her again and clears its offer');
  check(JSON.stringify(data.tracked) === JSON.stringify([{ id: 'ada', tracked: false }, { id: 'ada', tracked: true }]), 'the opt-out and its undo each sent one request', data.tracked);
  const back = data.people.get('ada');
  check(JSON.stringify({ ...back, updated_at: original.updated_at }) === JSON.stringify(original), 'the profile is back as it was, apart from its version', back);

  await page.reload();
  check(await page.waitFor(`!!document.querySelector('#person-record-ada')`, 60_000) && await settled(page), 'People opens after a reload');
  const reloaded = await page.evaluate(`({ name: document.querySelector('#person-record-ada')?.textContent.includes('Ada Synthetic'), untracked: ${CARD('ada')}?.textContent.includes('Not tracked'), undo: !!document.querySelector('[data-people-undo]') })`);
  evidence.reloaded = reloaded;
  check(reloaded.name && !reloaded.untracked, 'after a reload the profile keeps its name and tracking', reloaded);
  check(!reloaded.undo, 'a reload drops the undo offer, as recorded (receipts live in memory)', reloaded);
  check(data.blocked.length === 0, 'no other write was attempted', data.blocked);
} catch (error) {
  check(false, 'the run finished without an exception', String(error.stack ?? error));
} finally {
  check(page.unexpected.length === 0, 'no request left the machine', page.unexpected);
  check(page.runtimeErrors.length === 0, 'no uncaught page error', page.runtimeErrors);
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
