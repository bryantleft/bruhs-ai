import assert from 'node:assert/strict';
import { createHash, X509Certificate } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';

const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_MODULE || 'playwright');
const fixture = JSON.parse(await readFile(process.argv[2], 'utf8'));
const output = process.env.BRUHS_MOBILE_QUESTION_FIXTURE;
await mkdir(output, { recursive: true });
const certificate = new X509Certificate(await readFile(fixture.certificate));
const spki = createHash('sha256').update(certificate.publicKey.export({ type: 'spki', format: 'der' })).digest('base64');
const browser = await chromium.launch({
  executablePath: process.env.BRUHS_CHROME,
  args: [`--ignore-certificate-errors-spki-list=${spki}`],
});
try {
  const page = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true });
  page.setDefaultTimeout(16000);
  await page.goto(fixture.pairing);
  await page.getByLabel('Device name').fill('Question fixture phone');
  await page.getByRole('button', { name: 'Pair this device' }).click();
  await page.getByRole('button', { name: /Phone question fixture/ }).click();
  const heading = page.getByRole('button', { name: 'Question from the agent' });
  await heading.waitFor();
  const send = page.getByRole('button', { name: 'Send answers', exact: true });
  assert(await send.isDisabled());
  const reject = await page.evaluate(async ({ thread }) => {
    const headers = { Authorization: `Bearer ${localStorage.getItem('bruhs.mobile.device.v1')}` };
    const { value: snapshot } = await (await fetch(`/api/mobile/thread?thread_id=${thread}`, { headers })).json();
    const review = snapshot.question;
    assertFixture(review);
    const hash = review.review_hash;
    const response = await fetch('/api/mobile/command', {
      method: 'POST',
      headers: { ...headers, 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: crypto.randomUUID(), sequence: 1, action: {
        kind: 'answer_question', thread_id: thread, turn_generation: review.turn_generation,
        request_id: review.request_id, review_hash: (hash[0] === '0' ? '1' : '0') + hash.slice(1),
        reply: { type: 'skip' },
      } }),
    });
    return (await response.json()).outcome.status;
    function assertFixture(question) {
      if (!question || Object.hasOwn(question, 'input')) throw new Error('Missing structured review');
    }
  }, { thread: fixture.thread });
  assert.equal(reject, 'rejected');
  await page.reload();
  await page.getByRole('button', { name: /Phone question fixture/ }).click();
  await heading.waitFor();
  await page.screenshot({ path: join(output, 'phone-question.png') });
  const choice = page.getByRole('button', { name: /Concise/ });
  const questionPreview = page.locator('.question-preview').first();
  assert.equal(await questionPreview.locator('strong').textContent(), 'Sample');
  assert.equal(await questionPreview.locator('code').textContent(), 'startup');
  const external = questionPreview.getByRole('link', { name: 'Read the guide' });
  assert.equal(await external.getAttribute('href'), 'https://example.invalid/notes');
  assert.equal(await external.getAttribute('rel'), 'noopener noreferrer');
  assert.equal(await questionPreview.getByRole('link', { name: 'Blocked destination' }).count(), 0);
  assert.equal(await questionPreview.locator('script, iframe, img').count(), 0);
  await external.evaluate(link => link.addEventListener('click', event => event.preventDefault(), { once: true }));
  await external.click();
  assert.equal(await choice.getAttribute('aria-pressed'), 'false');
  await page.getByLabel('Custom answer').fill('Keep this answer while reading');
  await questionPreview.scrollIntoViewIfNeeded();
  await page.screenshot({ path: join(output, 'phone-question-markdown.png') });
  const linkResponse = page.waitForResponse(response => response.url().includes('/api/mobile/question-link?'));
  await questionPreview.getByRole('button', { name: 'Project example' }).click();
  const firstLinkPage = await linkResponse;
  assert.equal(firstLinkPage.status(), 200);
  const firstText = (await firstLinkPage.json()).value;
  assert(firstText.text.startsWith('Linked project example'));
  assert(!firstText.text.includes('First line before the anchor'));
  const denied = await page.evaluate(async ({ route, other }) => {
    const url = new URL(route);
    const unauthorized = await fetch(url, { headers: { Authorization: `Bearer ${other}` } });
    const token = localStorage.getItem('bruhs.mobile.device.v1');
    const headers = { Authorization: `Bearer ${token}` };
    const link = url.searchParams.get('link');
    url.searchParams.set('link', 'a'.repeat(64));
    const unknown = await fetch(url, { headers });
    url.searchParams.set('link', link);
    url.searchParams.set('review_hash', 'f'.repeat(64));
    const stale = await fetch(url, { headers });
    return [unauthorized.status, unknown.status, stale.status];
  }, { route: firstLinkPage.url(), other: fixture.read_only_device });
  assert.deepEqual(denied, [403, 409, 409]);
  const examplePath = join(dirname(process.argv[2]), 'answer-example.md');
  const original = await readFile(examplePath, 'utf8');
  await writeFile(examplePath, original.replace('Last example line', 'Edited example line'));
  await page.getByRole('button', { name: 'Next page' }).click();
  await page.getByText('This file changed. Reload the first page.').waitFor();
  await writeFile(examplePath, original);
  await page.getByRole('button', { name: '← Thread' }).click();
  await heading.waitFor();
  await questionPreview.getByRole('button', { name: 'Project example' }).click();
  await page.getByRole('button', { name: 'Next page' }).click();
  await page.getByText(/Last example line/).waitFor();
  await page.getByRole('button', { name: '← Thread' }).click();
  await heading.waitFor();
  assert.equal(await page.getByLabel('Custom answer').inputValue(), 'Keep this answer while reading');
  assert.equal(await choice.getAttribute('aria-pressed'), 'false');
  await questionPreview.getByRole('button', { name: 'Private file' }).click();
  await page.getByText('This link cannot be previewed as a project text file on the phone.').waitFor();
  await page.getByRole('button', { name: '← Thread' }).click();
  await heading.waitFor();
  await page.locator('#prompt').fill('Keep this separate phone draft');
  const picker = page.getByLabel('Attach files', { exact: true });
  assert.equal(await page.getByRole('button', { name: 'Attach files', exact: true }).count(), 1);
  await page.route('**/api/mobile/question-file?**', route => route.fulfill({
    status: 503, json: { error: 'Upload interrupted. Try again.' },
  }), { times: 1 });
  await picker.setInputFiles({ name: 'discard.txt', mimeType: 'text/plain', buffer: Buffer.from('do not send') });
  await page.getByRole('button', { name: 'Retry', exact: true }).waitFor();
  assert(await send.isDisabled());
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await page.getByText('Ready', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Remove discard.txt' }).click();
  await page.getByText('discard.txt', { exact: true }).waitFor({ state: 'detached' });
  const response = page.waitForResponse(result => result.url().includes('/api/mobile/question-file?') && result.request().method() === 'POST');
  await picker.setInputFiles({ name: 'release-notes.txt', mimeType: 'text/plain', buffer: Buffer.alloc(70_000, 'n') });
  const staged = await response;
  assert.equal(staged.status(), 200);
  assert.deepEqual(await staged.json(), { ok: true });
  await page.getByText('Ready', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Setup', exact: true }).click();
  await page.getByRole('button', { name: 'Examples', exact: true }).click();
  assert(await send.isEnabled(), 'A file can complete an answer without text or a choice');
  await page.getByText('release-notes.txt', { exact: true }).evaluate(element => element.scrollIntoView({ block: 'center' }));
  for (const scheme of ['dark', 'light']) {
    await page.emulateMedia({ colorScheme: scheme });
    const contrast = await page.evaluate(() => {
      const luminance = color => color.match(/\d+/g).slice(0, 3).map(Number).map(channel => {
        const srgb = channel / 255;
        return srgb <= 0.04045 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
      }).reduce((sum, channel, index) => sum + channel * [0.2126, 0.7152, 0.0722][index], 0);
      const ratio = (color, background) => {
        const [a, b] = [luminance(color), luminance(background)].sort((x, y) => y - x);
        return (a + 0.05) / (b + 0.05);
      };
      const row = document.querySelector('.question-file');
      const status = getComputedStyle(row.querySelector('small'));
      const surface = getComputedStyle(document.querySelector('.approval'));
      const action = getComputedStyle(document.querySelector('.approval .primary'));
      return [ratio(status.color, surface.backgroundColor), ratio(action.color, action.backgroundColor)];
    });
    assert(contrast.every(ratio => ratio >= 4.5), `${scheme} contrast ${contrast}`);
    await page.screenshot({ path: join(output, `phone-question-file-${scheme}.png`) });
  }
  await page.emulateMedia({ colorScheme: 'dark' });
  const checks = await page.evaluate(async ({ path, other, readOnly }) => {
    const headers = { 'Content-Type': 'application/octet-stream' };
    const stage = (target, token) => fetch(target, {
      method: 'POST', headers: { ...headers, Authorization: `Bearer ${token}` }, body: 'wrong bytes',
    });
    const owned = localStorage.getItem('bruhs.mobile.device.v1');
    const edit = new URL(path);
    const original = edit.searchParams.get('upload_id');
    const collision = await stage(edit, other);
    const removal = await fetch(edit, { method: 'DELETE', headers: { Authorization: `Bearer ${other}` } });
    const reviewPath = new URL(edit);
    edit.searchParams.set('upload_id', crypto.randomUUID());
    const forbidden = await stage(edit, readOnly);
    edit.searchParams.set('question', '1');
    const predefined = await stage(edit, owned);
    edit.searchParams.set('question', '0');
    edit.searchParams.set('review_hash', 'f'.repeat(64));
    const stale = await stage(edit, owned);
    const otherHeaders = { Authorization: `Bearer ${other}`, 'Content-Type': 'application/json' };
    const otherSession = await (await fetch('/api/mobile/session', { headers: otherHeaders })).json();
    const attemptedAnswer = await fetch('/api/mobile/command', {
      method: 'POST', headers: otherHeaders,
      body: JSON.stringify({ id: crypto.randomUUID(), sequence: otherSession.next_sequence, action: {
        kind: 'answer_question', thread_id: reviewPath.searchParams.get('thread_id'),
        turn_generation: Number(reviewPath.searchParams.get('turn_generation')),
        request_id: reviewPath.searchParams.get('request_id'), review_hash: reviewPath.searchParams.get('review_hash'),
        reply: { type: 'answer', answers: [{ selected: [], text: '', attachments: [original] }, { selected: [0], text: '' }] },
      } }),
    });
    return { collision: collision.status, removal: removal.status, attempt: (await attemptedAnswer.json()).outcome.status,
      forbidden: forbidden.status, predefined: predefined.status, stale: stale.status, file: Boolean(original) };
  }, { path: staged.url(), other: fixture.other_device, readOnly: fixture.read_only_device });
  assert.deepEqual(checks, { collision: 400, removal: 200, attempt: 'accepted', forbidden: 403, predefined: 400, stale: 409, file: true });
  await page.getByText('The answer could not be delivered. Try again.', { exact: true }).waitFor();
  await page.getByText('Ready', { exact: true }).waitFor();
  await page.getByRole('button', { name: /Concise/ }).click();
  await page.getByLabel('Custom answer').fill('Mention the startup fix');
  await heading.click();
  await heading.click();
  await send.click();
  await page.getByText('Question answered on phone.', { exact: true }).waitFor();
  assert.equal(await heading.count(), 0);
  assert.equal(await page.locator('#prompt').inputValue(), 'Keep this separate phone draft');
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
} finally {
  await browser.close();
}
