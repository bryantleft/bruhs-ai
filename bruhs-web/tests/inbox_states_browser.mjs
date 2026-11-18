// Inbox acceptance on the real WASM bundle and the browser's IndexedDB,
// through headless Chrome and CDP (cdp_harness.mjs). Synthetic imported
// conversations only; no request reaches a server or leaves the machine.
//
//   focus      Triage buttons keep keyboard focus on the same node, whose
//              label changes in place (Mark read, Important, Tomorrow,
//              Archive, by Enter and by pointer), and so do Save draft and
//              Command Enter in the reply box.
//   INBOX-09   Delete asks first with the shared wording; Cancel and Escape
//              keep the conversation and return focus to Delete; Delete
//              removes it, a reload does not bring it back, and the other
//              conversations are untouched.
//   INBOX-06   A reply draft saved while IndexedDB refuses writes stays in the
//              reply box and in the open conversation, the failure is shown,
//              Retry save stores it, and a reload keeps it.
//   INBOX-13   Loading (the database open is slowed), storage failure (the
//              open fails with VersionError against a newer database) and
//              Retry, and the Email view's failure with no server.
//   INBOX-11   Where focus lands after the import form's Cancel (pointer,
//              Enter, Escape) and after a conversation's Back; the
//              accessibility tree (Accessibility.getFullAXTree, role and
//              name) of the Inbox list, a conversation, the import form and
//              the delete dialog.
//
//   (cd bruhs-web && trunk build --dist ../target/web-dist)
//   node bruhs-web/tests/inbox_states_browser.mjs [dist]
// BRUHS_QA_DIR sets the output directory, BRUHS_WEB_AX_DIR where the
// accessibility trees go (default: <output>/web-ax), BRUHS_QA_PORT and
// BRUHS_QA_CHROME_PORT the static server's and Chrome's ports.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertFreshDist, checklist, launchChrome, newContext, newPage, serveDist, sleep, subtree } from './cdp_harness.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.argv[2] ?? process.env.BRUHS_WEB_DIST ?? path.join(root, 'target/web-dist'));
const startedAt = new Date().toISOString();
const output = path.resolve(process.env.BRUHS_QA_DIR ?? path.join(root, 'target/qa/inbox-states', startedAt.replace(/[:.]/g, '-')));
const axDir = path.resolve(process.env.BRUHS_WEB_AX_DIR ?? path.join(output, 'web-ax'));
const port = Number(process.env.BRUHS_QA_PORT ?? 39353);
const chromePort = Number(process.env.BRUHS_QA_CHROME_PORT ?? 9353);
const sources = [
  'bruhs-web/src/components/inbox_page.rs', 'bruhs-web/src/components/connected_inbox.rs',
  'bruhs-web/src/components/metadata_status.rs', 'bruhs-web/src/task_saves.rs',
  'bruhs-web/src/storage.rs', 'bruhs-web/src/ui/dialog.rs', 'bruhs-web/src/workflow_executor.rs',
  'bruhs-models/src/models/inbox.rs',
];
const freshness = await assertFreshDist(root, dist, sources);
await mkdir(output, { recursive: true });
await mkdir(axDir, { recursive: true });

// The confirmation's wording is shared with desktop; read it from the source.
const inboxModel = await readFile(path.join(root, 'bruhs-models/src/models/inbox.rs'), 'utf8');
const constant = name => {
  const match = inboxModel.match(new RegExp(`pub const ${name}: &str =\\s*"([^"]*)";`));
  assert(match, `bruhs-models has no ${name}`);
  return match[1];
};
const DELETE_TITLE = constant('DELETE_CONVERSATION_TITLE');
const DELETE_DETAIL = constant('DELETE_CONVERSATION_DETAIL');

const git = args => { try { return execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim(); } catch { return null; } };
const index = await readFile(path.join(dist, 'index.html'));
const report = {
  harness: 'inbox_states_browser.mjs', startedAt, dist, output, axDir,
  revision: git(['rev-parse', 'HEAD']), dirtyFiles: git(['status', '--porcelain'])?.split('\n').filter(Boolean) ?? null,
  indexSha256: createHash('sha256').update(index).digest('hex'), freshness,
  viewport: '1280x800', scenarios: {}, checks: [],
};

const BUTTON = text => `[...document.querySelectorAll('button')].find(b => b.textContent.trim() === ${JSON.stringify(text)} && b.offsetParent !== null)`;
const LABELED = label => `document.querySelector(${JSON.stringify(`[aria-label="${label}"]`)})`;
const ROW = sender => `[...document.querySelectorAll('[data-inbox-row]')].find(row => row.textContent.includes(${JSON.stringify(sender)}))`;
const text = selector => `(document.querySelector(${JSON.stringify(selector)})?.textContent ?? '').trim().replace(/\\s+/g, ' ')`;

const server = await serveDist(dist, port);
const chrome = await launchChrome({ port: chromePort, profile: path.join(root, 'target/qa/chrome-inbox-states') });
report.chrome = chrome.version;
report.origin = server.origin;
const check = checklist('setup');

// The thread list's loading strip shows for a moment after every load and
// then goes, moving the page up; wait for it so a pointer lands where aimed.
const settled = (page, timeout = 15_000) => page.waitFor(`!document.querySelector('[aria-label="Thread list loading status"]')`, timeout);

async function openInbox(page) {
  await page.goto(`${server.origin}/#/inbox`);
  const ready = await page.waitFor(`!!${BUTTON('Import conversation')} && !!${LABELED('Search inbox')}`, 60_000);
  return ready && settled(page);
}

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
  // The opened conversation is marked read and saved.
  await waitForRecord(page, id, record => record && !record.unread);
  return id;
}

async function backToInbox(page) {
  assert((await page.click(LABELED('Back to Inbox'))).clicked, 'Back to Inbox is clickable');
  assert(await page.waitFor(`!!${LABELED('Search inbox')} && !document.querySelector('[data-inbox-detail]')`), 'the list is back');
}

async function records(page) {
  const all = await page.readStore('inbox_conversations');
  return new Map(all.map(record => [record.id, record]));
}

async function waitForRecord(page, id, accept, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    last = (await records(page)).get(id);
    if (accept(last)) return last;
    await sleep(100);
  }
  return last;
}

const draftOf = record => [...(record?.messages ?? [])].reverse().find(message => message.send_state === 'draft')?.body ?? '';

async function saveTree(page, name, scope) {
  const nodes = await page.accessibilityTree();
  const scoped = subtree(nodes, scope);
  const file = path.join(axDir, `${name}.json`);
  await writeFile(file, `${JSON.stringify({ source: 'bruhs-web', capturedAt: new Date().toISOString(), revision: report.revision, viewport: report.viewport, url: await page.evaluate('location.hash'), method: 'CDP Accessibility.getFullAXTree; ignored, generic and empty text nodes left out', scope: scoped.map(({ depth, ...node }) => ({ ...node, depth })), page: nodes }, null, 2)}\n`);
  return { file, scoped: scoped.length, page: nodes.length };
}

const scenario = async (name, body) => {
  check.scenario = name;
  const context = await newContext(chrome.browser);
  const page = await newPage(chrome.browser, server.origin, { context, width: 1280, height: 800 });
  const evidence = report.scenarios[name] = { screenshots: [] };
  const shot = async label => { evidence.screenshots.push(await page.screenshot(path.join(output, `${name}-${label}.png`))); };
  try {
    await body(page, evidence, shot);
  } catch (error) {
    check(false, `scenario finished without an exception`, String(error.stack ?? error));
  } finally {
    check(page.unexpected.length === 0, 'no request left the machine', page.unexpected);
    check(page.runtimeErrors.length === 0, 'no uncaught page error', page.runtimeErrors);
    evidence.refusedApiRequests = page.refusedApi.length;
    evidence.consoleErrors = page.consoleErrors.slice(0, 20);
    await page.close();
    await context.dispose();
  }
};

try {
  await scenario('focus', async (page, evidence, shot) => {
    check(await openInbox(page), 'the Inbox opens');
    const id = await importConversation(page, { sender: 'Ada Example', subject: 'Launch plan', message: 'Can you confirm the launch date?' });
    // Each press must leave focus on the same button, whose label changes.
    // Pairs, because Archive also marks read and ends a snooze: each change
    // by Enter, then its undo by pointer.
    const steps = [
      { press: 'Mark unread', then: 'Mark read', input: 'Enter', saved: record => record.unread === true },
      { press: 'Mark read', then: 'Mark unread', input: 'pointer', saved: record => record.unread === false },
      { press: 'Important', then: 'Unstar', input: 'Enter', saved: record => record.labels.includes('important') },
      { press: 'Unstar', then: 'Important', input: 'pointer', saved: record => !record.labels.includes('important') },
      { press: 'Tomorrow', then: 'Unsnooze', input: 'Enter', saved: record => record.snoozed_until_ms > Date.now() },
      { press: 'Unsnooze', then: 'Tomorrow', input: 'pointer', saved: record => record.snoozed_until_ms === null },
      { press: 'Archive', then: 'Unarchive', input: 'Enter', saved: record => record.archived === true },
      { press: 'Unarchive', then: 'Archive', input: 'pointer', saved: record => record.archived === false },
    ];
    evidence.steps = [];
    for (const step of steps) {
      const tagged = await page.evaluate(`(() => { window.__probe = null; const el = ${BUTTON(step.press)}; if (!el) return false; window.__probe = el; el.focus(); return document.activeElement === el; })()`);
      if (!tagged) { check(false, `${step.press} is offered`, await page.evaluate(`[...document.querySelectorAll('[data-inbox-detail] header button')].map(b => b.textContent.trim())`)); continue; }
      if (step.input === 'Enter') await page.press('Enter');
      else await page.click('window.__probe');
      const relabeled = await page.waitFor(`window.__probe.textContent.trim() === ${JSON.stringify(step.then)}`, 5000);
      const after = await page.evaluate(`({ survived: window.__probe.isConnected, focused: document.activeElement === window.__probe, active: document.activeElement === document.body ? 'body' : document.activeElement.textContent.trim().slice(0, 40), visible: window.__probe.matches(':focus-visible') })`);
      const saved = await waitForRecord(page, id, record => record && step.saved(record));
      evidence.steps.push({ ...step, saved: undefined, tagged, relabeled, ...after });
      check(tagged && relabeled && after.survived && after.focused, `${step.press} by ${step.input} keeps focus on the same button, now ${step.then}`, after);
      check(saved && step.saved(saved), `${step.press} is saved`);
    }
    // Save draft: the button keeps focus; the draft is stored.
    await page.evaluate(`document.querySelector('textarea[aria-label="Reply draft"]').focus()`);
    await page.type('A draft kept in place');
    const draftTagged = await page.evaluate(`(() => { const el = ${BUTTON('Save draft')}; window.__probe = el; el.focus(); return document.activeElement === el; })()`);
    await page.press('Enter');
    const draftSaved = await waitForRecord(page, id, record => draftOf(record) === 'A draft kept in place');
    const draftAfter = await page.evaluate(`({ survived: window.__probe.isConnected, focused: document.activeElement === window.__probe, value: document.querySelector('textarea[aria-label="Reply draft"]').value })`);
    check(draftTagged && draftAfter.survived && draftAfter.focused, 'Save draft by Enter keeps focus on Save draft', draftAfter);
    check(draftOf(draftSaved) === 'A draft kept in place' && draftAfter.value === 'A draft kept in place', 'the draft is stored and stays in the reply box', draftAfter);
    // Command Enter in the reply box logs the reply; the box keeps focus.
    const boxTagged = await page.evaluate(`(() => { const el = document.querySelector('textarea[aria-label="Reply draft"]'); window.__probe = el; el.focus(); el.setSelectionRange(el.value.length, el.value.length); return document.activeElement === el; })()`);
    await page.press('Enter', 4);
    const logged = await waitForRecord(page, id, record => record?.messages.some(message => message.direction === 'outgoing' && message.send_state !== 'draft' && message.body === 'A draft kept in place'));
    const boxAfter = await page.evaluate(`({ survived: window.__probe.isConnected, focused: document.activeElement === window.__probe, value: window.__probe.value, shown: [...document.querySelectorAll('[data-inbox-history] article')].some(a => a.textContent.includes('A draft kept in place')) })`);
    check(boxTagged && boxAfter.survived && boxAfter.focused && boxAfter.value === '', 'Command Enter logs the reply and the reply box keeps focus, emptied', boxAfter);
    check(!!logged && boxAfter.shown, 'the logged reply is stored and shown in the history', boxAfter);
    evidence.draft = { draftAfter, boxAfter };
    await shot('after-triage');
  });

  await scenario('INBOX-09', async (page, evidence, shot) => {
    check(await openInbox(page), 'the Inbox opens');
    const keepA = await importConversation(page, { sender: 'Ada Keep', subject: 'Keep one', message: 'First conversation stays.' });
    await backToInbox(page);
    const doomed = await importConversation(page, { sender: 'Bea Delete', subject: 'Delete me', message: 'This one is deleted.' });
    await backToInbox(page);
    const keepC = await importConversation(page, { sender: 'Cy Keep', subject: 'Keep two', message: 'Third conversation stays.' });
    await backToInbox(page);
    const before = await records(page);
    check(before.size === 3, 'three conversations are stored', [...before.keys()]);
    assert((await page.click(ROW('Bea Delete'))).clicked, 'the row opens');
    check(await page.waitFor(`location.hash === ${JSON.stringify(`#/inbox/${doomed}`)} && !!document.querySelector('[data-inbox-detail]')`), 'the conversation to delete is open');
    const requestsBefore = page.refusedApi.length;

    // Pointer: Delete asks first, with the shared wording.
    assert((await page.click(BUTTON('Delete'))).clicked, 'Delete is clickable');
    check(await page.waitFor(`!!document.querySelector('[data-inbox-delete-confirm]')`), 'Delete asks first');
    const dialog = await page.evaluate(`(() => { const d = document.querySelector('[role=dialog]'); return { label: d?.getAttribute('aria-label'), title: document.querySelector('[data-inbox-delete-confirm] h2')?.textContent.trim(), detail: document.querySelector('[data-inbox-delete-confirm] p')?.textContent.trim(), focused: document.activeElement?.getAttribute('aria-label') ?? document.activeElement?.textContent.trim(), buttons: [...document.querySelectorAll('[data-inbox-delete-confirm] button')].map(b => b.getAttribute('aria-label') + '=' + b.textContent.trim()) }; })()`);
    evidence.dialog = dialog;
    check(dialog.title === DELETE_TITLE && dialog.detail === DELETE_DETAIL && dialog.label === DELETE_TITLE, 'the confirmation uses the shared title and detail', dialog);
    await shot('confirm');
    check((await (await records(page)).get(doomed)) !== undefined, 'nothing is deleted before confirming');

    // Cancel keeps it and returns focus to Delete.
    assert((await page.click(LABELED('Cancel deletion'))).clicked, 'Cancel is clickable');
    check(await page.waitFor(`!document.querySelector('[data-inbox-delete-confirm]')`), 'Cancel closes the confirmation');
    await sleep(200);
    let focus = await page.focused();
    evidence.afterCancel = focus;
    check(focus.text === 'Delete', 'Cancel returns focus to Delete', focus);
    check(!!(await page.evaluate(`!!document.querySelector('[data-inbox-detail]')`)) && (await records(page)).has(doomed), 'Cancel keeps the conversation');

    // Keyboard: Enter on Delete opens it; Escape keeps the conversation.
    await page.press('Enter');
    check(await page.waitFor(`!!document.querySelector('[data-inbox-delete-confirm]')`), 'Enter on Delete asks first');
    focus = await page.focused();
    evidence.openedByEnterFocus = focus;
    check(focus.label === 'Cancel deletion', 'a confirmation opened from the keyboard focuses its first control, Cancel', focus);
    await page.press('Tab');
    const tabbed = await page.focused();
    await page.press('Tab');
    const wrapped = await page.focused();
    evidence.dialogTabOrder = [focus.label, tabbed.label, wrapped.label];
    check(tabbed.label === 'Confirm deletion' && wrapped.label === 'Cancel deletion', 'Tab stays inside the confirmation', evidence.dialogTabOrder);
    await page.press('Escape');
    check(await page.waitFor(`!document.querySelector('[data-inbox-delete-confirm]')`), 'Escape closes the confirmation');
    await sleep(200);
    focus = await page.focused();
    evidence.afterEscape = focus;
    check(focus.text === 'Delete', 'Escape returns focus to Delete', focus);
    check((await records(page)).has(doomed), 'Escape keeps the conversation');

    // Delete removes it, and only it.
    assert((await page.click(BUTTON('Delete'))).clicked, 'Delete is clickable again');
    await page.waitFor(`!!document.querySelector('[data-inbox-delete-confirm]')`);
    assert((await page.click(LABELED('Confirm deletion'))).clicked, 'Confirm deletion is clickable');
    check(await page.waitFor(`location.hash === '#/inbox' && !!${LABELED('Search inbox')}`), 'Delete returns to the Inbox');
    const deadline = Date.now() + 10_000;
    let after = await records(page);
    while (after.has(doomed) && Date.now() < deadline) { await sleep(100); after = await records(page); }
    check(!after.has(doomed), 'the conversation is deleted from storage');
    check(JSON.stringify(after.get(keepA)) === JSON.stringify(before.get(keepA)) && JSON.stringify(after.get(keepC)) === JSON.stringify(before.get(keepC)), 'the other conversations are unchanged');
    const rows = await page.evaluate(`[...document.querySelectorAll('[data-inbox-row]')].map(row => row.textContent.includes('Bea Delete') ? 'deleted' : 'kept')`);
    check(rows.length === 2 && !rows.includes('deleted'), 'the list shows the other two', rows);
    const writes = page.refusedApi.slice(requestsBefore).filter(entry => !entry.startsWith('GET'));
    check(writes.length === 0, 'deleting sends nothing to a server', writes);
    await shot('deleted');

    await page.reload();
    check(await page.waitFor(`!!${LABELED('Search inbox')} && document.querySelectorAll('[data-inbox-row]').length === 2`, 60_000), 'after a reload the list has two conversations');
    const reloaded = await records(page);
    check(!reloaded.has(doomed) && reloaded.size === 2, 'a reload does not bring it back');
    check(JSON.stringify(reloaded.get(keepA)) === JSON.stringify(before.get(keepA)) && JSON.stringify(reloaded.get(keepC)) === JSON.stringify(before.get(keepC)), 'the other conversations are unchanged after the reload');
    await page.goto(`${server.origin}/#/inbox/${doomed}`);
    check(await page.waitFor(`document.body.innerText.includes('Conversation not found')`, 20_000), 'its link reads Conversation not found');
    await shot('reloaded-link');
  });

  await scenario('INBOX-06', async (page, evidence, shot) => {
    check(await openInbox(page), 'the Inbox opens');
    const id = await importConversation(page, { sender: 'Dee Draft', subject: 'Draft test', message: 'Please reply when you can.' });
    const body = 'Saved while storage refuses writes';
    await page.evaluate(`document.querySelector('textarea[aria-label="Reply draft"]').focus()`);
    await page.type(body);
    await page.evaluate(`window.__qa.refuseWrites = true`);
    assert((await page.click(BUTTON('Save draft'))).clicked, 'Save draft is clickable');
    const shown = await page.waitFor(`(() => { const notice = ${LABELED('Unsaved local changes')}; return !!notice && notice.textContent.includes('Not saved'); })()`, 10_000);
    check(shown, 'the failed save is shown');
    const failure = await page.evaluate(`(() => { const notice = ${LABELED('Unsaved local changes')}; return { alert: notice?.querySelector('[role=alert]')?.textContent.trim(), retry: !!notice && [...notice.querySelectorAll('button')].some(b => b.textContent.trim() === 'Retry save' && !b.disabled), refused: window.__qa.refusedWrites, value: document.querySelector('textarea[aria-label="Reply draft"]').value }; })()`);
    evidence.failure = failure;
    check(failure.refused > 0, 'IndexedDB refused the write', failure.refused);
    check(!!failure.alert, 'the failure says why', failure.alert);
    check(failure.retry, 'Retry save is offered');
    check(failure.value === body, 'the draft stays in the reply box', failure.value);
    check(draftOf((await records(page)).get(id)) === '', 'nothing was stored');
    await shot('failed');
    await backToInbox(page);
    check(!!(await page.evaluate(`!!${LABELED('Unsaved local changes')}`)), 'the failure stays shown on the list');
    assert((await page.click(ROW('Dee Draft'))).clicked, 'the row reopens');
    check(await page.waitFor(`document.querySelector('textarea[aria-label="Reply draft"]')?.value === ${JSON.stringify(body)}`), 'reopened, the conversation still has the draft');
    await page.evaluate(`window.__qa.refuseWrites = false`);
    assert((await page.click(`[...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'Retry save')`)).clicked, 'Retry save is clickable');
    check(await page.waitFor(`!${LABELED('Unsaved local changes')}`, 10_000), 'Retry save clears the failure');
    const stored = await waitForRecord(page, id, record => draftOf(record) === body);
    check(draftOf(stored) === body, 'Retry stores the draft');
    await shot('retried');
    await page.reload();
    check(await page.waitFor(`document.querySelector('textarea[aria-label="Reply draft"]')?.value === ${JSON.stringify(body)}`, 60_000), 'a reload keeps the draft');
    await shot('reloaded');
  });

  await scenario('INBOX-13', async (page, evidence, shot) => {
    check(await openInbox(page), 'the Inbox opens');
    await importConversation(page, { sender: 'Eve Loading', subject: 'Loading state', message: 'Shown after the slow open.' });
    // Loading: the database open is slowed by six seconds.
    const slow = await page.addInitScript('window.__qa.openDelayMs = 6000;');
    await page.goto(`${server.origin}/#/inbox`);
    await page.reload();
    const loading = await page.waitFor(`document.body.innerText.includes('Loading conversations…')`, 20_000);
    evidence.loading = await page.evaluate(`(() => { const section = ${LABELED('Conversations')}; return { status: section?.querySelector('[role=status]')?.textContent.trim(), detail: section?.querySelector('[role=status] + p')?.textContent.trim(), delayedOpens: window.__qa.delayedOpens }; })()`);
    check(loading && evidence.loading.status === 'Loading conversations…', 'a slow open shows Loading conversations…', evidence.loading);
    check(evidence.loading.detail === 'Your conversations will appear after local records load.', 'it says what happens next', evidence.loading.detail);
    await shot('loading');
    check(await page.waitFor(`!!${ROW('Eve Loading')}`, 20_000), 'the conversation appears once the open finishes');
    await page.removeInitScript(slow);
    await settled(page);

    // A provider failure: the Email view with no server.
    assert((await page.click(BUTTON('Email'))).clicked, 'Email is clickable');
    const provider = await page.waitFor(`[...document.querySelectorAll('[data-connected-inbox] [data-mail-feedback] [role=alert]')].some(alert => alert.textContent.trim().length > 0)`, 20_000);
    evidence.provider = await page.evaluate(`({ alerts: [...document.querySelectorAll('[data-connected-inbox] [role=alert]')].map(a => a.textContent.trim()).filter(Boolean), statuses: [...document.querySelectorAll('[data-connected-inbox] [role=status]')].map(s => s.textContent.trim()).filter(Boolean).slice(0, 6), offered: [...document.querySelectorAll('[data-connected-inbox] button')].filter(b => !b.disabled).map(b => b.textContent.trim()) })`);
    check(provider, 'the Email view shows a failure when the server cannot be reached', evidence.provider);
    check(evidence.provider.offered.includes('Refresh') && evidence.provider.offered.includes('Imported conversations'), 'it offers Refresh and the way back to imported conversations', evidence.provider.offered);
    await shot('provider-failure');
  });

  await scenario('INBOX-13-storage', async (page, evidence, shot) => {
    // A newer database (version 999999999) makes the app's open fail with
    // VersionError, as when a newer build has opened it.
    await page.goto(`${server.origin}/__qa_blank`);
    await page.waitFor(`document.readyState === 'complete'`);
    await page.evaluate(`new Promise((resolve, reject) => { const open = indexedDB.open('bruhs', 999999999); open.onsuccess = () => { open.result.close(); resolve(); }; open.onerror = () => reject(String(open.error)); })`);
    await page.goto(`${server.origin}/#/inbox`);
    const failed = await page.waitFor(`!!${LABELED('Inbox loading status')}`, 60_000);
    evidence.threadListSettled = await settled(page, 5_000);
    evidence.failure = await page.evaluate(`(() => { const status = ${LABELED('Inbox loading status')}; const section = ${LABELED('Conversations')}; return { alert: status?.querySelector('[role=alert]')?.textContent.trim(), retry: !!status && [...status.querySelectorAll('button')].some(b => b.textContent.trim() === 'Retry loading records'), list: section?.querySelector('[role=status]')?.textContent.trim(), listDetail: section?.querySelector('[role=status] + p')?.textContent.trim(), importOffered: !!${BUTTON('Import conversation')} }; })()`);
    check(failed && evidence.failure.alert === 'Local records could not be loaded. Existing storage was left unchanged; conversation changes made here are kept.', 'a failed open says the records could not be loaded', evidence.failure);
    check(evidence.failure.retry, 'Retry loading records is offered');
    check(evidence.failure.list === 'Conversations could not be loaded', 'the list says it could not load', evidence.failure.list);
    check(evidence.failure.listDetail === 'Stored conversations were left unchanged.', 'the list says nothing was replaced, not that records are loading', evidence.failure.listDetail);
    await shot('storage-failure');
    const version = await page.evaluate(`indexedDB.databases().then(list => list.find(db => db.name === 'bruhs')?.version ?? null)`);
    check(version === 999999999, 'the newer database was left unchanged', version);
    // Recovery: once the newer database is gone, Retry loads.
    await page.evaluate(`new Promise((resolve, reject) => { const del = indexedDB.deleteDatabase('bruhs'); del.onsuccess = () => resolve(); del.onerror = () => reject(String(del.error)); del.onblocked = () => reject('blocked'); })`);
    assert((await page.click(BUTTON('Retry loading records'))).clicked, 'Retry loading records is clickable');
    check(await page.waitFor(`!${LABELED('Inbox loading status')} && document.body.innerText.includes('Import a message or choose another view.')`, 20_000), 'Retry loads the empty Inbox');
    evidence.recovered = await page.evaluate(`(() => { const section = ${LABELED('Conversations')}; return section?.querySelector('[role=status]')?.textContent.trim(); })()`);
    await shot('storage-retried');
  });

  await scenario('INBOX-11', async (page, evidence, shot) => {
    check(await openInbox(page), 'the Inbox opens');
    await importConversation(page, { sender: 'Fay Focus', subject: 'Keyboard pass', message: 'Opened from the keyboard.' });
    await backToInbox(page);
    await importConversation(page, { sender: 'Gus Focus', subject: 'Second row', message: 'Another conversation.' });
    await backToInbox(page);
    evidence.trees = {};
    evidence.trees.inboxList = await saveTree(page, 'inbox-list', node => node.role === 'main');
    await shot('list');

    // The import form's Cancel, by pointer.
    assert((await page.click(BUTTON('Import conversation'))).clicked, 'Import conversation is clickable');
    await page.waitFor(`!!document.querySelector('[role=dialog] input[aria-label=Sender]')`);
    evidence.importOpenedFocus = await page.focused();
    evidence.trees.importForm = await saveTree(page, 'import-form', node => node.role === 'dialog');
    await shot('import-form');
    assert((await page.click(`[...document.querySelectorAll('[role=dialog] button')].find(b => b.textContent.trim() === 'Cancel')`)).clicked, 'Cancel is clickable');
    await page.waitFor(`!document.querySelector('[role=dialog]')`);
    await sleep(200);
    evidence.afterPointerCancel = await page.focused();
    check(evidence.afterPointerCancel.text === 'Import conversation', 'pointer Cancel returns focus to Import conversation', evidence.afterPointerCancel);

    // By keyboard: Enter opens it, Tab to Cancel, Enter.
    await page.evaluate(`${BUTTON('Import conversation')}.focus()`);
    await page.press('Enter');
    await page.waitFor(`!!document.querySelector('[role=dialog] input[aria-label=Sender]')`);
    evidence.importByEnterFocus = await page.focused();
    let tabs = 0;
    while (tabs < 20 && (await page.evaluate(`document.activeElement?.textContent.trim() !== 'Cancel'`))) { await page.press('Tab'); tabs += 1; }
    evidence.tabsToCancel = tabs;
    await page.press('Enter');
    await page.waitFor(`!document.querySelector('[role=dialog]')`);
    await sleep(200);
    evidence.afterKeyboardCancel = await page.focused();
    check(evidence.afterKeyboardCancel.text === 'Import conversation', 'keyboard Cancel returns focus to Import conversation', evidence.afterKeyboardCancel);

    // Escape.
    await page.evaluate(`${BUTTON('Import conversation')}.focus()`);
    await page.press('Enter');
    await page.waitFor(`!!document.querySelector('[role=dialog] input[aria-label=Sender]')`);
    await page.press('Escape');
    await page.waitFor(`!document.querySelector('[role=dialog]')`);
    await sleep(200);
    evidence.afterEscape = await page.focused();
    check(evidence.afterEscape.text === 'Import conversation', 'Escape returns focus to Import conversation', evidence.afterEscape);

    // A conversation opened from the keyboard, and Back.
    await page.evaluate(`${ROW('Fay Focus')}.focus()`);
    await page.press('Enter');
    check(await page.waitFor(`!!document.querySelector('[data-inbox-detail]')`), 'Enter on a row opens it');
    await sleep(300);
    evidence.openedConversationFocus = await page.focused();
    evidence.trees.conversation = await saveTree(page, 'conversation', node => node.role === 'main');
    await shot('conversation');

    // The delete dialog.
    await page.evaluate(`${BUTTON('Delete')}.focus()`);
    await page.press('Enter');
    await page.waitFor(`!!document.querySelector('[data-inbox-delete-confirm]')`);
    evidence.trees.deleteDialog = await saveTree(page, 'delete-dialog', node => node.role === 'dialog');
    await shot('delete-dialog');
    await page.press('Escape');
    await page.waitFor(`!document.querySelector('[data-inbox-delete-confirm]')`);

    await page.evaluate(`${LABELED('Back to Inbox')}.focus()`);
    await page.press('Enter');
    await page.waitFor(`!!${LABELED('Search inbox')}`);
    await sleep(300);
    evidence.afterBack = await page.focused();
    for (const [name, tree] of Object.entries(evidence.trees)) check(tree.scoped > 0, `the ${name} tree was saved`, tree);
  });
} finally {
  report.finishedAt = new Date().toISOString();
  report.checks = check.all;
  report.summary = check.summary();
  await writeFile(path.join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  await chrome.close();
  await server.close();
  console.log(JSON.stringify({ ...report.summary, output, axDir }));
  process.exitCode = report.summary.failed === 0 ? 0 : 1;
}
