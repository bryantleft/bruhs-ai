import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { join } from 'node:path';

const [input, output] = process.argv.slice(2);
const fixture = JSON.parse(await readFile(input, 'utf8'));
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const browser = await chromium.launch({ headless: true });
const checks = [], errors = [];
const report = { passed: false, checks, errors, profiles: [] };
const check = (name, actual, expected) => {
  assert.deepEqual(actual, expected, name);
  checks.push(name);
};
const parentCard = page => page.locator(`.card:has([data-card="${fixture.parent.id}"])`);
const drawerSettled = async page => {
  await page.waitForFunction(() => {
    const drawer = document.querySelector('#drawer');
    return drawer && !drawer.hidden && getComputedStyle(drawer).opacity === '1';
  });
};
const openParent = async page => {
  await page.locator(`[data-card="${fixture.parent.id}"]`).click();
  await page.locator('#drawer-title').getByText(fixture.parent.title, { exact: true }).waitFor();
  await drawerSettled(page);
};
const reviewRegion = page => page.getByRole('region', { name: 'Agent verification', exact: true });
const newPage = async options => {
  const page = await browser.newPage(options);
  page.setDefaultTimeout(fixture.timeout_ms);
  page.on('pageerror', error => errors.push(error.message));
  return page;
};

try {
  const page = await newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(`${fixture.base}/api/coordination/ui#task=${fixture.parent.id}`);
  await reviewRegion(page).getByText('Reviewed specification revision 1.', { exact: true }).waitFor();
  await drawerSettled(page);
  check('Completed verifier has an action status requiring review', await page.locator('#detail .status').innerText(), 'Review ready');
  check('Review ready uses the action tone', await page.locator('#detail .status.tone-action').count(), 1);
  check('Agent review remains in Validating', await page.locator('#detail .facts').getByText('Validating', { exact: true }).count(), 1);
  check('Card counts agent reviews without claiming checks passed', await parentCard(page).getByText('1 agent review', { exact: true }).count(), 1);
  check('Review explains that the report findings need inspection', await reviewRegion(page).innerText().then(text => text.includes('Inspect its findings and cited check results')), true);
  await reviewRegion(page).getByRole('button', { name: 'Open verification report', exact: true }).click();
  await page.locator('#drawer-title').getByText(fixture.review.title, { exact: true }).waitFor();
  check('Opening the report exposes unresolved findings', await page.locator('.transcript').innerText().then(text => text.includes('FAIL: synthetic regression check')), true);
  check('Report identifies its evidence as an agent review', await page.locator('#detail').innerText().then(text => text.includes('Agent verification report.')), true);
  await page.locator('#detail').getByRole('button', { name: fixture.parent.title, exact: true }).click();
  await reviewRegion(page).waitFor();
  await drawerSettled(page);
  await page.screenshot({ path: join(output, 'desktop-review-ready.png') });

  await page.getByRole('button', { name: 'Attach specification', exact: true }).click();
  await page.locator('#spec').selectOption(fixture.spec.id);
  await page.locator('#spec-content').fill('The implementation passes its regression checks and handles the newly reported edge case.');
  const [savedResponse] = await Promise.all([
    page.waitForResponse(response => response.url().endsWith('/api/coordination/commands') && response.request().method() === 'POST'),
    page.getByRole('button', { name: 'Save specification', exact: true }).click(),
  ]);
  check('Owner specification edit is accepted', savedResponse.status(), 200);
  await parentCard(page).getByText('Review outdated', { exact: true }).waitFor();
  await page.locator('#drawer-close').click();
  await openParent(page);
  check('Specification edit changes the report status', await page.locator('#detail .status').innerText(), 'Review outdated');
  check('Outdated report returns the card to Building', await page.locator('#detail .facts').getByText('Building', { exact: true }).count(), 1);
  check('Outdated report explains the changed specification', await reviewRegion(page).innerText().then(text => text.includes('The specification changed after this review.')), true);
  check('Old report keeps its reviewed revision visible', await reviewRegion(page).getByText('Reviewed specification revision 1.', { exact: true }).count(), 1);
  await page.screenshot({ path: join(output, 'desktop-review-outdated.png') });

  await page.locator('#child-actions summary').click();
  await page.locator('#child-title').fill('Review revision two');
  await page.locator('#child-prompt').fill('FIXTURE_CASE=review_findings');
  await page.getByRole('button', { name: 'Verify against specification', exact: true }).click();
  await reviewRegion(page).getByText('Reviewed specification revision 2.', { exact: true }).waitFor();
  await parentCard(page).getByText('Review ready', { exact: true }).waitFor();
  const snapshotResponse = await page.request.get(`${fixture.base}/api/coordination`);
  const snapshot = await snapshotResponse.json();
  const newest = snapshot.tasks.find(task => task.title === 'Review revision two');
  check('Fresh verification is a separate completed run', newest.state, 'completed');
  check('Fresh verification pins the new specification', newest.spec_revision, 2);
  check('Fresh verification keeps read-only access', newest.permission_mode, 'plan');
  check('Fresh verifier can finish with unresolved findings', newest.messages.at(-1).text.includes('FAIL:'), true);
  check('Original report remains available after re-review', snapshot.tasks.some(task => task.id === fixture.review.id), true);
  const board = await page.request.get(`${fixture.base}/api/coordination/board`).then(response => response.json());
  const card = board.cards.find(card => card.id === fixture.parent.id);
  check('Card links to the newest report', card.verification.task_id, newest.id);
  check('Both verification runs remain counted', card.verifications, 2);
  report.profiles.push('desktop: real HTTP, report navigation, specification save, separate verification');

  const mobile = await newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  await mobile.goto(`${fixture.base}/api/coordination/ui#task=${fixture.parent.id}`);
  await reviewRegion(mobile).getByText('Reviewed specification revision 2.', { exact: true }).waitFor();
  check('Mobile presents the same review status', await mobile.locator('#detail .status').innerText(), 'Review ready');
  check('Mobile document fits its viewport', await mobile.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  check('Mobile review section wraps within the drawer', await reviewRegion(mobile).evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await reviewRegion(mobile).getByRole('button', { name: 'Open verification report', exact: true }).tap();
  await mobile.locator('#drawer-title').getByText(newest.title, { exact: true }).waitFor();
  check('Mobile touch navigation exposes findings', await mobile.locator('.transcript').innerText().then(text => text.includes('FAIL: synthetic regression check')), true);
  await mobile.locator('.drawer-body').evaluate(element => { element.scrollTop = 0; });
  await mobile.screenshot({ path: join(output, 'mobile-verification-report.png') });
  await mobile.keyboard.press('Escape');
  check('Escape closes the report panel', await mobile.locator('#drawer').isVisible(), false);
  await mobile.locator(`[data-card="${fixture.parent.id}"]`).focus();
  await mobile.keyboard.press('Enter');
  await reviewRegion(mobile).waitFor();
  check('Keyboard reopens the task review', await mobile.locator('#drawer-title').innerText(), fixture.parent.title);
  report.profiles.push('mobile: touch, keyboard, wrapping and report navigation');

  const host = await newPage({ viewport: { width: 1280, height: 900 } });
  await host.goto(`${fixture.base}/api/workbench/ui#task=${fixture.parent.id}`);
  const frame = host.frameLocator('#panel');
  await reviewRegion(frame).getByText('Reviewed specification revision 2.', { exact: true }).waitFor();
  check('Workbench host embeds the same review status', await frame.locator('#detail .status').innerText(), 'Review ready');
  await reviewRegion(frame).getByRole('button', { name: 'Open verification report', exact: true }).click();
  await frame.locator('#drawer-title').getByText(newest.title, { exact: true }).waitFor();
  check('Embedded Workbench report exposes unresolved findings', await frame.locator('.transcript').innerText().then(text => text.includes('FAIL: synthetic regression check')), true);
  check('Embedded report navigation stays inside Workbench', new URL(host.url()).pathname, '/api/workbench/ui');
  report.profiles.push('Workbench host: shared embedded task surface and report navigation');

  check('No browser script errors', errors, []);
  report.passed = true;
} catch (error) {
  report.failure = error.stack || String(error);
  throw error;
} finally {
  await writeFile(join(output, 'browser-report.json'), JSON.stringify(report, null, 2) + '\n');
  await browser.close();
  console.log(JSON.stringify(report));
}
