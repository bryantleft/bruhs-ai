// Settings, Vault: late answers and stuck calls, on the real WASM bundle
// through headless Chrome and CDP (cdp_harness.mjs). The vault routes are
// answered by in-process fixtures with Key entries only, so no value is
// ever typed or sent; every other /api request is refused, and nothing
// leaves the machine.
//
//   BC M1  A Delete, Check or Save answer that lands late moves focus only
//          when focus is still on the Vault page (or on nothing), as the
//          desktop does: an owner who moved on to another Settings
//          section keeps their place.
//   BC M2  A vault call gets the desktop's 75 second limit. A call that
//          never answers is dropped, its buttons come back, and the page
//          says the service couldn't be reached. The limit is shortened
//          here by stretching only the page's 75 second timer.
//
//   (cd bruhs-web && trunk build --dist ../target/qa/vault-web-dist)
//   node bruhs-web/tests/vault_settings_browser.mjs [dist]
// BRUHS_QA_DIR sets the output directory; BRUHS_QA_PORT and
// BRUHS_QA_CHROME_PORT the static server's and Chrome's ports.
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertFreshDist, checklist, launchChrome, newContext, newPage, serveDist, sleep } from './cdp_harness.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.argv[2] ?? process.env.BRUHS_WEB_DIST ?? path.join(root, 'target/qa/vault-web-dist'));
const startedAt = new Date().toISOString();
const output = path.resolve(process.env.BRUHS_QA_DIR ?? path.join(root, 'target/qa/vault-settings', startedAt.replace(/[:.]/g, '-')));
const port = Number(process.env.BRUHS_QA_PORT ?? 39371);
const chromePort = Number(process.env.BRUHS_QA_CHROME_PORT ?? 9371);
const freshness = await assertFreshDist(root, dist, [
  'bruhs-web/src/components/vault_settings.rs', 'bruhs-web/src/api/vault.rs', 'bruhs-models/src/vault.rs',
]);
await mkdir(output, { recursive: true });

// Copy the page must show (bruhs_models::vault).
const UNREACHABLE = "Bruhs couldn't reach its service. Check that it's running, then refresh.";
const key = (id, name) => ({ id, name, scope: { type: 'global' }, kind: { kind: 'value' }, state: 'saved', attention: null });
let entries = [];
// Vault changes and checks wait here until the run lets one answer.
const waiting = [];
const requests = [];
function api({ method, url }) {
  const at = url.pathname;
  if (at.endsWith('/stream')) return 'stream';
  if (at === '/api/vault' && method === 'GET') return { body: { entries, bruhs: [], one_password: false } };
  if (at.startsWith('/api/vault/')) {
    requests.push(`${method} ${at}`);
    return new Promise(resolve => waiting.push({ method, at, resolve }));
  }
  return undefined;
}

// Only the vault's 75 second timer is stretched, and only once a check
// asks for it; every other timer runs as written.
const TIMERS = `(() => {
  const original = window.setTimeout;
  window.__vaultTimers = [];
  window.setTimeout = function (callback, delay, ...rest) {
    if (delay === 75000) {
      window.__vaultTimers.push(delay);
      if (window.__vaultDeadlineMs) delay = window.__vaultDeadlineMs;
    }
    return original.call(window, callback, delay, ...rest);
  };
})()`;

const check = checklist('vault-settings');
const server = await serveDist(dist, port);
const chrome = await launchChrome({ port: chromePort, profile: path.join(root, 'target/qa/chrome-vault-settings') });
const report = { harness: 'vault_settings_browser.mjs', startedAt, dist, output, freshness, chrome: chrome.version, evidence: {} };
let page;
try {
  const context = await newContext(chrome.browser);
  page = await newPage(chrome.browser, server.origin, { context, width: 1280, height: 900, api, initScripts: [TIMERS] });

  const open = async () => {
    await page.goto(`${server.origin}/#/settings`);
    const ready = await page.waitFor(`!!document.querySelector('#vault [data-vault-new]') && document.querySelectorAll('#vault [data-vault-row]').length === ${entries.length}`, 60_000);
    if (!ready) throw new Error(`Settings, Vault never listed ${entries.length} entries`);
  };
  const row = id => `document.querySelector('#vault [data-vault-row="${id}"]')`;
  const answer = async (match, reply) => {
    const ok = await (async () => {
      for (let tries = 0; tries < 100; tries++) {
        const index = waiting.findIndex(item => match(item));
        if (index >= 0) { waiting.splice(index, 1)[0].resolve(reply); return true; }
        await sleep(50);
      }
      return false;
    })();
    if (!ok) throw new Error('the page never sent the call to answer');
    await sleep(400);
  };
  // Another Settings section: the nav's Calendar button, outside the page.
  const elsewhere = `[...document.querySelectorAll('nav[aria-label="Settings sections"] button')].find(button => button.textContent.trim() === 'Calendar')`;
  const focusElsewhere = () => page.evaluate(`(() => { const target = ${elsewhere}; target.focus(); return document.activeElement === target; })()`);
  const focusIs = expression => page.evaluate(`document.activeElement === (${expression})`);
  const newEntry = `document.querySelector('#vault [data-vault-new]')`;

  check.scenario = 'BC M1';
  entries = [key('v1-a', 'Alpha'), key('v1-b', 'Bravo'), key('v1-c', 'Charlie'), key('v1-d', 'Delta')];
  await open();

  // Delete, focus left on the page: the answer moves it to New entry.
  await page.click(`${row('v1-a')}.querySelector('[data-vault-delete]')`);
  await page.click(`${row('v1-a')}.querySelector('[data-vault-confirm]')`);
  entries = entries.filter(entry => entry.id !== 'v1-a');
  await answer(item => item.method === 'DELETE' && item.at === '/api/vault/v1-a', { status: 204, body: null });
  check(await page.evaluate(`!${row('v1-a')}`), 'a delete answered removes its row');
  check(await focusIs(newEntry), 'a delete answered while focus stayed on the page moves focus to New entry', await page.focused());

  // Delete, then the owner moves to another section before the answer.
  await page.click(`${row('v1-b')}.querySelector('[data-vault-delete]')`);
  await page.click(`${row('v1-b')}.querySelector('[data-vault-confirm]')`);
  check(await focusElsewhere(), 'focus moved to another Settings section while the delete waits');
  entries = entries.filter(entry => entry.id !== 'v1-b');
  await answer(item => item.method === 'DELETE' && item.at === '/api/vault/v1-b', { status: 204, body: null });
  check(await page.evaluate(`!${row('v1-b')}`), 'the late delete still removes its row');
  check(await focusIs(elsewhere), 'a late delete answer leaves focus in the other section', await page.focused());

  // Check on an entry that is gone answers 404: the row goes, focus stays.
  await page.click(`${row('v1-c')}.querySelector('[data-vault-check]')`);
  check(await focusElsewhere(), 'focus moved to another Settings section while the check waits');
  entries = entries.filter(entry => entry.id !== 'v1-c');
  await answer(item => item.method === 'POST' && item.at === '/api/vault/v1-c/check', { status: 404, body: { error: 'That vault entry is gone.' } });
  check(await page.evaluate(`!${row('v1-c')}`), 'a check that finds the entry gone removes its row');
  check(await focusIs(elsewhere), 'a late check answer leaves focus in the other section', await page.focused());

  // Save on an edit (the Key's value left empty keeps it).
  const edit = `${row('v1-d')}.querySelector('[data-vault-edit]')`;
  const save = `${row('v1-d')}.querySelector('[data-vault-form] button[type="submit"]')`;
  await page.click(edit);
  await page.waitFor(`!!${save}`);
  await page.click(save);
  check(await focusElsewhere(), 'focus moved to another Settings section while the save waits');
  await answer(item => item.method === 'PUT' && item.at === '/api/vault/v1-d', { status: 200, body: key('v1-d', 'Delta') });
  check(await page.evaluate(`!${row('v1-d')}.querySelector('[data-vault-form]')`), 'the late save closes its form');
  check(await focusIs(elsewhere), 'a late save answer leaves focus in the other section', await page.focused());
  await page.click(edit);
  await page.waitFor(`!!${save}`);
  await page.click(save);
  await answer(item => item.method === 'PUT' && item.at === '/api/vault/v1-d', { status: 200, body: key('v1-d', 'Delta') });
  check(await focusIs(edit), 'a save answered while focus stayed on the page returns focus to Edit', await page.focused());

  check.scenario = 'BC M2';
  await page.evaluate('window.__vaultDeadlineMs = 1500');
  const checkButton = `${row('v1-d')}.querySelector('[data-vault-check]')`;
  await page.click(checkButton);
  check(await page.evaluate(`${checkButton}.disabled`), 'the buttons wait while the check runs');
  await sleep(500);
  check(await page.evaluate(`${checkButton}.disabled`), 'and keep waiting while the server says nothing');
  const back = await page.waitFor(`!${checkButton}.disabled`, 5_000);
  check(back, 'past the limit the buttons come back');
  const alert = await page.evaluate(`${row('v1-d')}.querySelector('[role="alert"]')?.textContent.trim() ?? null`);
  check(alert === UNREACHABLE, 'and the row says the service could not be reached', alert);
  const timers = await page.evaluate('window.__vaultTimers.length');
  check(timers > 0, "the page asked for the desktop's 75 second limit", timers);
  report.evidence.requests = requests;
  check(page.runtimeErrors.length === 0, 'no page errors', page.runtimeErrors);
  check(page.unexpected.length === 0, 'nothing left the machine', page.unexpected);
  await page.screenshot(path.join(output, 'vault-after-timeout.png'));
} finally {
  await page?.close();
  await chrome.close();
  await server.close();
}
report.checks = check.all;
report.summary = check.summary();
await writeFile(path.join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
console.log(`${report.summary.passed} passed, ${report.summary.failed} failed; report in ${output}`);
process.exit(report.summary.failed === 0 ? 0 : 1);
