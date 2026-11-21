// Real Delivery HTTP/UI acceptance in an owned private HOME and Git fixture.
// The Python owner installs a fail-closed Git proxy and forge stubs before launch.
import assert from 'node:assert/strict';
import { readFile, open, rename } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';

const [input, output] = process.argv.slice(2);
const fixture = JSON.parse(await readFile(input, 'utf8'));
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const browser = await chromium.launch({ headless: true });
const report = { passed: false, checks: [], failures: [], errors: [], console: [], requests: [], profiles: [], scope: 'Real shared Delivery UI and local HTTP/processes only; no delivery is published.' };
const check = (name, actual, expected = true) => {
  try {
    assert.deepEqual(actual, expected, name);
    report.checks.push({ name, passed: true, actual });
    return true;
  } catch (error) {
    report.checks.push({ name, passed: false, actual, expected });
    report.failures.push({ name, actual, expected, error: error.message });
    return false;
  }
};
const atomicWrite = async (path, content) => {
  const temporary = `${path}.qa-${process.pid}-${Date.now()}`;
  const handle = await open(temporary, 'wx');
  try { await handle.writeFile(content); await handle.sync(); } finally { await handle.close(); }
  await rename(temporary, path);
};
const rows = async path => existsSync(path) ? (await readFile(path, 'utf8')).trim().split('\n').filter(Boolean).map(line => JSON.parse(line)) : [];
const api = async (path, body, method = 'POST') => {
  const response = await fetch(`${fixture.base}/api/delivery${path}`, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined, signal: AbortSignal.timeout(fixture.timeout_ms) });
  const value = await response.json();
  assert.equal(response.status, 200, JSON.stringify(value));
  return value;
};
const settled = async id => {
  const deadline = Date.now() + fixture.timeout_ms;
  while (Date.now() < deadline) {
    const receipt = (await api('', undefined, 'GET')).actions.find(row => row.prepared.id === id);
    if (receipt && !['prepared', 'running'].includes(receipt.status)) return receipt;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw new Error(`Receipt ${id} did not settle within the fixture budget`);
};
const idle = page => page.waitForFunction(() => !document.getElementById('delivery').hasAttribute('aria-busy'));
const activate = async (locator, profile) => {
  if (profile.hasTouch) await locator.tap();
  else { await locator.focus(); await locator.press('Enter'); }
};
const preview = async (page, profile) => {
  const response = page.waitForResponse(response => response.url() === `${fixture.base}/api/delivery/prepare` && response.request().method() === 'POST');
  await activate(page.getByRole('button', { name: 'Preview action', exact: true }), profile);
  const result = await response;
  const body = await result.json();
  await idle(page);
  return { status: result.status(), body };
};
const refresh = async page => { await page.getByRole('button', { name: 'Refresh activity', exact: true }).click(); await idle(page); };
const receiptLocator = (page, id) => page.locator(`#receipts details[data-id="${id}"]`);
const reveal = async (locator, profile) => {
  await locator.locator('summary').waitFor();
  if (!(await locator.evaluate(node => node.open))) await activate(locator.locator('summary'), profile);
};
const evidence = async (page, profile, suffix) => {
  check(`${profile.name}: ${suffix} fits the viewport`, await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await page.screenshot({ path: join(output, `${profile.name}-${suffix}.png`), fullPage: true });
};

try {
  for (const profile of [
    { name: 'desktop', viewport: { width: 1440, height: 900 } },
    { name: 'mobile-touch', viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true },
  ]) {
    const page = await browser.newPage(profile);
    page.setDefaultTimeout(fixture.timeout_ms);
    page.on('pageerror', error => report.errors.push({ profile: profile.name, message: error.message }));
    page.on('console', message => { if (['warning', 'error'].includes(message.type())) report.console.push({ profile: profile.name, type: message.type(), text: message.text() }); });
    page.on('request', request => {
      if (request.url().startsWith(`${fixture.base}/api/delivery`) && request.method() === 'POST') report.requests.push({ profile: profile.name, url: request.url(), method: request.method(), body: request.postDataJSON() });
    });
    await page.route('**/*', route => {
      const url = new URL(route.request().url());
      if (url.origin === fixture.base) return route.continue();
      report.errors.push({ profile: profile.name, message: `Blocked unexpected external browser request to ${url.origin}` });
      return route.abort();
    });
    const prefix = profile.name;
    const profileResult = { name: prefix, completed: false, successful_id: null, failed_id: null, fatal_error: null };
    try {
      await atomicWrite(join(fixture.project, 'selected.txt'), 'GOOD\n');
      await page.goto(`${fixture.base}/api/delivery/ui?path=${encodeURIComponent(fixture.project)}`);
      await idle(page);
      await page.evaluate(() => document.fonts.ready);
      check(`${prefix}: shared UI Monaspace font loads from owned local assets`, await page.evaluate(() => document.fonts.check('14px Monaspace')));
      await page.locator('#receipts details').first().waitFor();
      check(`${prefix}: shared UI labels independent-check action`, await page.locator('#action option[value="verify"]').textContent(), 'Run independent checks');
      check(`${prefix}: restored completed proof is visible`, (await receiptLocator(page, fixture.successful_id).locator('summary').innerText()).startsWith('Checks passed for reviewed source'));
      check(`${prefix}: restored prepared checks remain awaiting review`, (await receiptLocator(page, fixture.pending_id).locator('summary').innerText()).startsWith('Awaiting review'));
      const details = page.locator('#action-details');
      await activate(details.locator(':scope > summary'), profile);
      check(`${prefix}: Prepare an action opens by ${profile.hasTouch ? 'touch' : 'keyboard'}`, await details.evaluate(node => node.open));
      await page.getByRole('combobox', { name: 'Action', exact: true }).selectOption('verify');
      await page.getByRole('combobox', { name: 'Forge', exact: true }).selectOption('git_hub');
      await page.getByLabel('Execution timeout, seconds', { exact: true }).fill('30');
      const filesInput = page.getByLabel('Selected files, one relative file per line', { exact: true });
      await filesInput.fill('');
      const processesBeforeErrors = (await rows(fixture.check_log)).length;
      const empty = await preview(page, profile);
      check(`${prefix}: empty selection returns an actionable error`, empty.status >= 400 && typeof empty.body.error === 'string' && empty.body.error.length > 0);
      check(`${prefix}: error is exposed through role alert`, await page.getByRole('alert').isVisible());
      check(`${prefix}: validation did not run commands`, (await rows(fixture.check_log)).length, processesBeforeErrors);
      await filesInput.fill('.GiT/config');
      const unsafePath = await preview(page, profile);
      check(`${prefix}: Git-internals selection is refused`, unsafePath.status >= 400 && /internals/i.test(unsafePath.body.error));
      await evidence(page, profile, 'selection-error');

      await atomicWrite(join(fixture.project, 'selected.txt'), 'FAIL\n');
      await filesInput.fill(fixture.files.join('\n'));
      const countBeforePrepare = (await rows(fixture.check_log)).length;
      const executeBeforePrepare = report.requests.filter(request => request.url.endsWith('/execute')).length;
      const failedPreview = await preview(page, profile);
      assert.equal(failedPreview.status, 200, JSON.stringify(failedPreview));
      profileResult.failed_id = failedPreview.body.id;
      await page.getByRole('button', { name: 'Run reviewed checks', exact: true }).waitFor();
      check(`${prefix}: browser preview prepares verify with exact selected files`, failedPreview.body.action, { type: 'verify', files: fixture.files });
      check(`${prefix}: reviewed commands match the configured frozen commands`, failedPreview.body.commands, fixture.commands);
      check(`${prefix}: preparation did not start a check process`, (await rows(fixture.check_log)).length, countBeforePrepare);
      check(`${prefix}: preparation did not call execute`, report.requests.filter(request => request.url.endsWith('/execute')).length, executeBeforePrepare);
      check(`${prefix}: preparation has no private checkout yet`, existsSync(failedPreview.body.workspace_path), false);
      await evidence(page, profile, 'reviewed-checks');
      await activate(page.getByRole('button', { name: 'Run reviewed checks', exact: true }), profile);
      await idle(page);
      const failed = await settled(failedPreview.body.id);
      check(`${prefix}: explicit browser execution observes real exit 23`, failed.status === 'failed' && /23/.test(failed.error) && failed.completed_commands < failed.prepared.commands.length);
      await refresh(page);
      const failedRow = receiptLocator(page, failedPreview.body.id);
      await reveal(failedRow, profile);
      check(`${prefix}: failed check is presented as Needs attention`, (await failedRow.locator('summary').innerText()).startsWith('Needs attention'));
      check(`${prefix}: precise failed-process error remains visible`, await failedRow.locator('p.error').innerText(), failed.error);
      check(`${prefix}: failed check cannot be used as delivery proof`, await failedRow.getByRole('button', { name: 'Use these checks for delivery', exact: true }).count(), 0);
      await evidence(page, profile, 'failed-check');

      await atomicWrite(join(fixture.project, 'selected.txt'), 'GOOD\n');
      const successPreview = await preview(page, profile);
      assert.equal(successPreview.status, 200, JSON.stringify(successPreview));
      profileResult.successful_id = successPreview.body.id;
      const countBeforeRun = (await rows(fixture.check_log)).length;
      await activate(page.getByRole('button', { name: 'Run reviewed checks', exact: true }), profile);
      await idle(page);
      const success = await settled(successPreview.body.id);
      check(`${prefix}: explicit browser execution runs all successful check commands`, success.status === 'completed' && success.error === null && success.completed_commands === success.prepared.commands.length);
      check(`${prefix}: completed check retains an immutable checked-tree identity`, typeof success.checked_tree === 'string' && /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/i.test(success.checked_tree));
      check(`${prefix}: real process logs match the reviewed command count`, (await rows(fixture.check_log)).length - countBeforeRun, success.prepared.commands.length);
      const observed = (await rows(fixture.check_log)).slice(countBeforeRun);
      check(`${prefix}: checks use private HEAD plus selected files only`, observed.every(row => row.cwd !== fixture.project && row.selected === 'GOOD\n' && row.tracked_unselected === 'HEAD ONLY\n' && !row.untracked_exists && !row.ignored_exists && !row.deleted_exists));
      check(`${prefix}: temporary check checkout is removed`, existsSync(successPreview.body.workspace_path), false);
      await refresh(page);
      const successRow = receiptLocator(page, successPreview.body.id);
      await reveal(successRow, profile);
      check(`${prefix}: completed receipt explicitly says checks passed for reviewed source`, (await successRow.locator('summary').innerText()).startsWith('Checks passed for reviewed source'));
      check(`${prefix}: completed check receipt avoids offering its deleted checkout as an issue workspace`, await successRow.getByRole('button', { name: 'Open issue workspace', exact: true }).count(), 0);
      await evidence(page, profile, 'completed-check');
      const beforeUseProcesses = (await rows(fixture.check_log)).length;
      const beforeUseRequests = report.requests.filter(request => /\/(prepare|execute)$/.test(request.url)).length;
      await activate(successRow.getByRole('button', { name: 'Use these checks for delivery', exact: true }), profile);
      await idle(page);
      check(`${prefix}: use completed checks selects commit/push/PR action`, await page.locator('#action').inputValue(), 'commit_push_pull');
      check(`${prefix}: use completed checks copies the reviewed selected files`, await filesInput.inputValue(), fixture.files.join('\n'));
      check(`${prefix}: use completed checks selects its completed proof`, await page.locator('#verification').inputValue(), successPreview.body.id);
      check(`${prefix}: use completed checks keeps publication explicit`, report.requests.filter(request => /\/(prepare|execute)$/.test(request.url)).length, beforeUseRequests);
      check(`${prefix}: use completed checks does not rerun checks`, (await rows(fixture.check_log)).length, beforeUseProcesses);
      await page.getByLabel('Pull request title', { exact: true }).fill(`Owned ${prefix} verified proposal`);
      await page.getByLabel('Commit message', { exact: true }).fill(`Owned ${prefix} verified proposal`);
      await page.getByLabel('Pull request description', { exact: true }).fill('Owned local browser fixture. No external delivery is permitted.');
      await page.getByLabel('Base branch', { exact: true }).fill('release');
      await page.getByLabel('Head branch', { exact: true }).fill('main');
      await filesInput.fill('selected.txt');
      check(`${prefix}: changing file selection clears a mismatched proof`, await page.locator('#verification').inputValue(), '');
      check(`${prefix}: changing file selection removes the previous completed run`, await page.locator(`#verification option[value="${successPreview.body.id}"]`).count(), 0);
      await filesInput.fill([...fixture.files].reverse().join('\n'));
      check(`${prefix}: exact file selection matches regardless of order`, await page.locator(`#verification option[value="${successPreview.body.id}"]`).count(), 1);
      await page.getByRole('combobox', { name: 'Independent check run', exact: true }).selectOption(successPreview.body.id);
      const deliveriesBefore = report.requests.filter(request => request.url.endsWith('/execute')).length;
      const delivery = await preview(page, profile);
      assert.equal(delivery.status, 200, JSON.stringify(delivery));
      check(`${prefix}: proposal carries host-copied completed proof`, delivery.body.verification.receipt_id, successPreview.body.id);
      check(`${prefix}: proposal proof binds the completed immutable tree`, delivery.body.verification.checked_tree, success.checked_tree);
      check(`${prefix}: proposal displays checked source and commands`, (await page.locator('#proposal').innerText()).includes('Independent checks passed for this reviewed source'));
      check(`${prefix}: delivery proposal requires explicit execution`, await page.getByRole('button', { name: 'Run reviewed action', exact: true }).isVisible());
      check(`${prefix}: preview has not executed delivery`, report.requests.filter(request => request.url.endsWith('/execute')).length, deliveriesBefore);
      await evidence(page, profile, 'proof-proposal');
      await activate(page.locator('#proposal').getByRole('button', { name: 'Dismiss', exact: true }), profile);
      await idle(page);

      await page.getByRole('combobox', { name: 'Independent check run', exact: true }).selectOption('');
      const noProof = await preview(page, profile);
      assert.equal(noProof.status, 200, JSON.stringify(noProof));
      check(`${prefix}: proposal without proof exposes no execution control`, await page.locator('#proposal').getByRole('button', { name: 'Run reviewed action', exact: true }).count(), 0);
      check(`${prefix}: proposal without proof offers independent-check review`, await page.getByRole('button', { name: 'Review independent checks', exact: true }).isVisible());
      await evidence(page, profile, 'proof-required');
      const countBeforeRecovery = (await rows(fixture.check_log)).length;
      const recoveryResponse = page.waitForResponse(response => response.url() === `${fixture.base}/api/delivery/prepare` && response.request().method() === 'POST');
      await activate(page.getByRole('button', { name: 'Review independent checks', exact: true }), profile);
      const recovery = await (await recoveryResponse).json();
      await idle(page);
      check(`${prefix}: proof recovery prepares checks for the same file selection`, recovery.action, { type: 'verify', files: [...fixture.files].reverse() });
      check(`${prefix}: proof recovery requires Run reviewed checks`, await page.getByRole('button', { name: 'Run reviewed checks', exact: true }).isVisible());
      check(`${prefix}: proof recovery has not run commands`, (await rows(fixture.check_log)).length, countBeforeRecovery);
      const noProofReceipt = (await api('', undefined, 'GET')).actions.find(row => row.prepared.id === noProof.body.id);
      check(`${prefix}: proof recovery dismisses the unproved delivery`, noProofReceipt.status, 'cancelled');
      await activate(page.locator('#proposal').getByRole('button', { name: 'Dismiss', exact: true }), profile);
      await idle(page);
      await page.getByRole('combobox', { name: 'Independent check run', exact: true }).selectOption(successPreview.body.id);
      await atomicWrite(join(fixture.project, 'selected.txt'), 'STALE BROWSER SOURCE\n');
      const stale = await preview(page, profile);
      check(`${prefix}: stale checked source is rejected during UI proposal preparation`, stale.status === 409 && /source changed/i.test(stale.body.error));
      check(`${prefix}: stale-check error is readable in the live alert`, await page.getByRole('alert').innerText(), stale.body.error);
      await evidence(page, profile, 'stale-proof-error');
      await atomicWrite(join(fixture.project, 'selected.txt'), 'GOOD\n');
      const beforeReload = (await rows(fixture.check_log)).length;
      await page.reload();
      await idle(page);
      await receiptLocator(page, successPreview.body.id).locator('summary').waitFor();
      check(`${prefix}: reload preserves completed checks`, (await receiptLocator(page, successPreview.body.id).locator('summary').innerText()).startsWith('Checks passed for reviewed source'));
      check(`${prefix}: reload never auto-runs prepared or completed checks`, (await rows(fixture.check_log)).length, beforeReload);
      if (profile.hasTouch) {
        await activate(page.locator('#action-details > summary'), profile);
        await page.locator('#action').selectOption('verify');
        const touchSizes = await page.locator('#action, #timeout, #action-form button[type="submit"]').evaluateAll(nodes => nodes.map(node => ({ id: node.id || node.textContent, height: node.getBoundingClientRect().height })));
        check(`${prefix}: check controls have at least 44px touch targets`, touchSizes.every(size => size.height >= 44));
        profileResult.touch_targets = touchSizes;
      } else {
        const summary = page.locator('#action-details > summary');
        await summary.focus();
        await summary.press('Enter');
        await page.locator('#action').selectOption('verify');
        await page.locator('#action').focus();
        await page.keyboard.press('Tab');
        check(`${prefix}: keyboard focus reaches selected-files input`, await page.evaluate(() => document.activeElement.id), 'files');
      }
      await evidence(page, profile, 'restored-proof');
      profileResult.completed = true;
    } catch (error) {
      profileResult.fatal_error = error.stack || error.message;
      report.failures.push({ name: `${prefix}: workflow exception`, error: profileResult.fatal_error });
      await page.screenshot({ path: join(output, `${prefix}-exception.png`), fullPage: true }).catch(() => {});
    } finally {
      await atomicWrite(join(fixture.project, 'selected.txt'), 'GOOD\n');
      report.profiles.push(profileResult);
      await atomicWrite(join(output, 'browser-report.json'), JSON.stringify(report, null, 2) + '\n');
      await page.close();
    }
  }
  check('No browser script errors or unexpected external requests', report.errors, []);
  const blocked = (await rows(fixture.git_log)).filter(row => !row.allowed);
  check('No mutating or unexpected Git command reached the fixture barrier', blocked, []);
  check('No forge client was invoked', existsSync(fixture.forge_log), false);
  report.passed = report.failures.length === 0 && report.profiles.every(profile => profile.completed);
} finally {
  await atomicWrite(join(output, 'browser-report.json'), JSON.stringify(report, null, 2) + '\n');
  await browser.close();
}
console.log(JSON.stringify({ passed: report.passed, checks: report.checks.length, failures: report.failures, profiles: report.profiles }));
process.exitCode = report.passed ? 0 : 1;
