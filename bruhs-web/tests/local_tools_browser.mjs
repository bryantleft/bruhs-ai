// Synthetic UI acceptance; real browser transport is covered by the core's live test.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import { createRequire } from 'node:module';
import path from 'node:path';

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const root = path.resolve(import.meta.dirname, '../..');
const html = await fs.readFile(path.join(root, 'bruhs-server/src/routes/local_tools_ui.html'));
const output = process.env.BRUHS_UI_OUTPUT || '/tmp/bruhs-ui-preview';
await fs.mkdir(output, { recursive: true });
const browser = await chromium.launch({ executablePath: process.env.BRUHS_BROWSER_BIN || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });

try {
  for (const viewport of [{ width: 1280, height: 800 }, { width: 640, height: 480 }, { width: 390, height: 844 }]) {
    const calls = [];
    let grants = [], browsers = [], pendingScreenshot;
    const server = http.createServer(async (request, response) => {
      let raw = '';
      for await (const chunk of request) raw += chunk;
      const data = raw ? JSON.parse(raw) : undefined;
      calls.push({ method: request.method, path: request.url, data });
      const json = (value, status = 200) => { response.writeHead(status, { 'Content-Type': 'application/json' }); response.end(JSON.stringify(value)); };
      if (request.url === '/') { response.writeHead(200, { 'Content-Type': 'text/html' }); response.end(html); return; }
      if (request.url === '/api/local-tools') return json({ grants, browsers });
      if (request.url === '/api/local-tools/grants' && request.method === 'POST') { const grant = { ...data, id: 'grant-1' }; grants.push(grant); return json(grant); }
      if (request.url === '/api/local-tools/browser/start') { const session = { ...data, id: 'browser-1' }; browsers.push(session); return json(session); }
      if (request.url === '/api/local-tools/browser/command') {
        if (data.command.action === 'screenshot') { pendingScreenshot = () => json({ error: 'Browser access was revoked' }, 403); return; }
        return json({ value: { action: data.command.action }, image: null });
      }
      if (request.url === '/api/local-tools/grants/grant-1' && request.method === 'DELETE') { grants = []; browsers = []; json({ revoked: true }); pendingScreenshot?.(); return; }
      json({ error: 'Unexpected test route' }, 404);
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const context = await browser.newContext({ viewport, isMobile: viewport.width === 390, hasTouch: viewport.width === 390, recordVideo: { dir: output, size: viewport } });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    if (viewport.width === 390) {
      const cdp = await context.newCDPSession(page);
      // Chrome DevTools Slow 3G profile, used only by this acceptance fixture.
      await cdp.send('Network.enable');
      await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 400, downloadThroughput: 400 * 1024 / 8, uploadThroughput: 200 * 1024 / 8 });
    }
    try {
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.waitForFunction(() => document.getElementById('status').textContent === 'Ready');
      assert.equal(calls.filter(call => call.method === 'POST').length, 0, 'opening the panel must never grant or launch tools');
      const principal = '<script>window.unexpectedExecution=true</script>' + 'a'.repeat(250);
      await page.locator('#principal').fill(principal);
      await page.getByRole('button', { name: 'Allow browser and open', exact: true }).click();
      await page.waitForFunction(() => document.querySelectorAll('#grants button').length === 1 && !document.getElementById('allow-browser').disabled);
      assert.equal(grants[0].principal, principal);
      assert.equal(await page.evaluate(() => window.unexpectedExecution), undefined);
      await page.locator('#selector').fill('#login');
      await page.locator('#text').fill(`a';window.bad=true;//`);
      await page.getByRole('button', { name: 'Fill element', exact: true }).click();
      await page.waitForFunction(() => document.getElementById('result').textContent.includes('fill'));
      assert.equal(calls.find(call => call.data?.command?.action === 'fill').data.command.text, `a';window.bad=true;//`);
      await page.locator('[data-browser="screenshot"]').click();
      await page.waitForFunction(() => document.getElementById('status').textContent === 'Reading browser');
      assert.equal(await page.getByRole('button', { name: 'Revoke', exact: true }).isEnabled(), true, 'revocation must remain available while a tool runs');
      await page.getByRole('button', { name: 'Revoke', exact: true }).click();
      await page.waitForFunction(() => document.getElementById('error').textContent.includes('revoked'));
      assert.equal(grants.length, 0);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'long labels must not overflow the viewport');
      assert.deepEqual(errors, []);
      await page.screenshot({ path: path.join(output, `local-tools-${viewport.width}.png`), fullPage: true });
    } finally {
      await context.close();
      await new Promise(resolve => server.close(resolve));
    }
  }
} finally { await browser.close(); }
process.stdout.write(`Local verification controls passed desktop, compact and touch/Slow 3G acceptance. Captures: ${output}\n`);
