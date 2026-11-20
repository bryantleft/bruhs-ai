import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { join } from 'node:path';

const [input, output] = process.argv.slice(2);
const fixture = JSON.parse(await readFile(input, 'utf8'));
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const browser = await chromium.launch({ headless: true });
const report = { passed: false, checks: [], errors: [], profiles: [] };
const check = (name, actual, expected) => {
  assert.deepEqual(actual, expected, name);
  report.checks.push(name);
};
try {
  for (const profile of [
    { name: 'desktop', viewport: { width: 1440, height: 900 } },
    { name: 'mobile', viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true },
  ]) {
    const page = await browser.newPage(profile);
    page.setDefaultTimeout(fixture.timeout_ms);
    page.on('pageerror', error => report.errors.push(error.message));
    await page.goto(`${fixture.base}/api/delivery/ui?path=${encodeURIComponent(fixture.project)}`);
    const receipt = page.locator(`#receipts details[data-id="${fixture.id}"]`);
    await receipt.locator('summary').waitFor();
    check(`${profile.name}: failed receipt needs attention`, (await receipt.locator('summary').innerText()).startsWith('Needs attention · 0/'), true);
    await receipt.locator('summary').click();
    check(`${profile.name}: receipt shows the precise source-change recovery`, await receipt.locator('p.error').innerText(), fixture.error);
    check(`${profile.name}: owner is directed to prepare a fresh review`, (await receipt.innerText()).includes('Prepare and review the action again.'), true);
    check(`${profile.name}: failed proposal cannot be executed again`, await receipt.getByRole('button', { name: 'Review action', exact: true }).count(), 0);
    check(`${profile.name}: receipt fits the viewport`, await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
    await page.screenshot({ path: join(output, `${profile.name}-source-guard.png`), fullPage: true });
    report.profiles.push(profile.name);
    await page.close();
  }
  check('No browser script errors', report.errors, []);
  report.passed = true;
} finally {
  await writeFile(join(output, 'browser-report.json'), JSON.stringify(report, null, 2) + '\n');
  await browser.close();
}
