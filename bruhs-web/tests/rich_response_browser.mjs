// Run after `trunk build`: node tests/rich_response_browser.mjs.
// Isolated browser storage, mocked APIs and media. No provider calls or user data.
import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.join(root, 'bruhs-web/dist');
const evidence = process.env.BRUHS_BROWSER_ARTIFACTS;
if (evidence) await mkdir(evidence, { recursive: true });
const modelsPath = path.join(root, 'bruhs-models/src/logic/rich_response.rs');
const source = await readFile(modelsPath, 'utf8');
const routeSource = await readFile(path.join(root, 'bruhs-server/src/routes/rich_response.rs'), 'utf8');
const appIndex = await readFile(path.join(root, 'bruhs-web/index.html'), 'utf8');
async function constant(name) {
  const start = source.indexOf(`pub const ${name}:`);
  assert.ok(start >= 0, `${name} must exist in production source`);
  const next = source.indexOf('pub const ', start + 1);
  const declaration = source.slice(start, next < 0 ? undefined : next);
  const pieces = [];
  for (const token of declaration.matchAll(/r#"([\s\S]*?)"#|include_str!\("([^"]+)"\)/g)) {
    pieces.push(token[1] ?? await readFile(path.resolve(path.dirname(modelsPath), token[2]), 'utf8'));
  }
  return pieces.join('');
}
const host = await constant('INTERACTIVE_HOST');
const mediaHost = await constant('MEDIA_HOST');
const runtime = await readFile(path.join(root, 'bruhs-models/src/logic/rich_response/widget-runtime.js'), 'utf8');
const policy = source.match(/pub const INTERACTIVE_CSP: &str = "([^"]+)";/)?.[1];
const hostPolicy = routeSource.match(/pub const HOST_CSP: &str = "([^"]+)";/)?.[1];
const mediaPolicy = routeSource.match(/const MEDIA_HOST_CSP: &str = "([^"]+)";/)?.[1];
const appPolicy = appIndex.match(/http-equiv="Content-Security-Policy"\s+content="([^"]+)"/)?.[1];
assert.ok(host && mediaHost && policy && hostPolicy && mediaPolicy && appPolicy);
async function playwrightModule() {
  if (process.env.BRUHS_PLAYWRIGHT_MODULE) return import(pathToFileURL(process.env.BRUHS_PLAYWRIGHT_MODULE).href);
  try { return await import('playwright'); } catch { /* Optional development dependency. */ }
  const cache = path.join(homedir(), '.npm/_npx');
  for (const entry of await readdir(cache).catch(() => [])) {
    try { return await import(pathToFileURL(path.join(cache, entry, 'node_modules/playwright/index.mjs')).href); }
    catch { /* Other cached npx packages need not contain Playwright. */ }
  }
  throw Error('Install Playwright or set BRUHS_PLAYWRIGHT_MODULE to its index.mjs.');
}
const { chromium } = await playwrightModule();
const chrome = process.env.BRUHS_CHROME ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined);
const pixel = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=', 'base64');
const wav = Buffer.alloc(16_044);
wav.write('RIFF'); wav.writeUInt32LE(wav.length - 8, 4); wav.write('WAVEfmt ', 8); wav.writeUInt32LE(16, 16);
wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22); wav.writeUInt32LE(8_000, 24); wav.writeUInt32LE(16_000, 28);
wav.writeUInt16LE(2, 32); wav.writeUInt16LE(16, 34); wav.write('data', 36); wav.writeUInt32LE(16_000, 40);
const video = spawnSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=red:s=16x16:r=5:d=1', '-an', '-c:v', 'libvpx', '-f', 'webm', 'pipe:1']);
const vtt = Buffer.from('WEBVTT\n\n00:00.000 --> 00:01.000\nCaption fixture\n');
const emptyState = () => ({ fields: {}, custom: null, scroll_x: 0, scroll_y: 0 });
const mediaBytes = name => name.endsWith('.webm') ? { body: video.stdout, type: 'video/webm' } : name.endsWith('.wav') ? { body: wav, type: 'audio/wav' } : name.endsWith('.vtt') ? { body: vtt, type: 'text/vtt' } : { body: pixel, type: 'image/png' };
let origin;
const probe = `<button id="counter" onclick="this.textContent=String(Number(this.textContent)+1);window.bruhs?.setState({count:Number(this.textContent)})">0</button>
<input id="state" aria-label="Widget value" value="initial"><input id="secret" type="password" aria-label="Private field">
<input id="check" type="checkbox" aria-label="Remember selection">
<button id="submit" onclick="window.bruhs.submit({label:'Use choice',message:'A reviewed widget draft'})">Propose draft</button>
<button id="network" onclick="fetch('__ORIGIN__/blocked-fetch').then(()=>this.textContent='allowed').catch(()=>this.textContent='blocked')">Test network</button>
<button id="navigation" onclick="location.href='__ORIGIN__/blocked-navigation?value=sample'">Test navigation</button>
<script>window.identity=crypto.randomUUID();
try{parent.document.body;window.parentDenied=false}catch{window.parentDenied=true}
try{top.document.body;window.topDenied=false}catch{window.topDenied=true}
try{localStorage.getItem('bruhs');window.storageDenied=false}catch{window.storageDenied=true}
window.bruhs?.ready.then(state=>{document.getElementById('counter').textContent=String(state?.count || 0)});
</script>`;
const documentFor = html => `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${policy}"><meta name="referrer" content="no-referrer"><style>body{margin:0;padding:12px;font:14px ui-monospace;box-sizing:border-box}*{box-sizing:border-box}</style><script>${runtime}</script></head><body>${html}</body></html>`;
const widgetUrl = html => `${origin}/api/agent/rich-response#${encodeURIComponent(JSON.stringify({ html: documentFor(html), nonce: 'fixture' }))}`;
const requests = [];
const server = createServer(async (req, res) => {
  const url = new URL(req.url, origin); requests.push(url.pathname);
  if (url.pathname === '/api/agent/rich-response') { res.writeHead(200, { 'Content-Type': 'text/html', 'Content-Security-Policy': hostPolicy }); res.end(host); return; }
  if (url.pathname === '/sandbox-test' || url.pathname === '/navigation-test') {
    const html = url.pathname === '/navigation-test' ? `<script>location.href='${origin}/automatic-navigation?value=sample'</script>` : probe.replaceAll('__ORIGIN__', origin);
    res.writeHead(200, { 'Content-Type': 'text/html' }); res.end(`<!doctype html><meta http-equiv="Content-Security-Policy" content="${appPolicy}"><iframe id="rich" sandbox="allow-scripts" src="${widgetUrl(html)}"></iframe>`); return;
  }
  if (url.pathname === '/favicon.ico') { res.writeHead(204); res.end(); return; }
  const file = path.resolve(dist, `.${url.pathname === '/' ? '/index.html' : decodeURIComponent(url.pathname)}`);
  if (!file.startsWith(`${dist}${path.sep}`)) { res.writeHead(403); res.end(); return; }
  try { res.writeHead(200, { 'Content-Type': { '.wasm': 'application/wasm', '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml' }[path.extname(file)] ?? 'application/octet-stream' }); res.end(await readFile(file)); }
  catch { res.writeHead(404); res.end('Missing browser fixture asset'); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true, executablePath: chrome });
const passed = [];
const check = (value, label) => { assert.ok(value, label); passed.push(label); if (process.env.BRUHS_BROWSER_DEBUG) console.log(label); };
async function eventually(predicate, message) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  throw Error(message);
}

try {
  const security = await browser.newPage();
  await security.goto(`${origin}/sandbox-test`);
  const widget = security.frameLocator('#rich').frameLocator('iframe');
  await widget.locator('#counter').click();
  check(await widget.locator('#counter').textContent() === '1', 'Inline scripts and event handlers work under the app CSP');
  const child = security.frames().find(frame => frame.url() === 'about:srcdoc');
  check(await child.evaluate(() => window.parentDenied && window.topDenied && window.storageDenied), 'Widgets cannot read parent DOM, top DOM, or local storage');
  check(await child.evaluate(() => ['camera', 'microphone', 'geolocation'].every(feature => !(document.permissionsPolicy ?? document.featurePolicy).allowsFeature(feature))), 'Widget device and location permissions stay denied');
  await widget.locator('#network').click(); await widget.locator('#network').filter({ hasText: 'blocked' }).waitFor();
  check(!requests.includes('/blocked-fetch'), 'Widget fetches are blocked before reaching the server');
  const clickedNavigation = security.waitForEvent('framenavigated', { predicate: frame => frame.url().startsWith('chrome-error:') });
  await widget.locator('#navigation').click(); await clickedNavigation;
  check(!requests.includes('/blocked-navigation'), 'Widget button navigation cannot exfiltrate data');
  const automaticNavigation = security.waitForEvent('framenavigated', { predicate: frame => frame.url().startsWith('chrome-error:') });
  await security.goto(`${origin}/navigation-test`); await automaticNavigation;
  check(!requests.includes('/automatic-navigation'), 'Automatic widget navigation cannot exfiltrate data');
  check(!requests.some(request => request.includes('%3C') || request.includes('onclick')), 'Widget source stays outside server request URLs');
  await security.close();

  if (!process.argv.includes('--sandbox-only')) {
    const context = await browser.newContext({ viewport: { width: 1100, height: 850 }, acceptDownloads: true });
    const states = new Map(), assets = new Map(), aliases = new Map(), diagnostics = new Map();
    const captures = [], exports = [], remoteCaptures = [];
    let remoteRequests = 0, bypassRequests = 0, activeWrites = 0, maxWrites = 0, activeReports = 0, maxReports = 0;
    let failSaves = false, originalsAvailable = true, apiDelay = 25;
    const artifactId = '00000000-0000-4000-8000-000000000001';
    const artifactHtml = '<input id="published-state" value="artifact"><script>window.bruhs.ready.then(()=>{throw Error("fixture artifact runtime failure")})</script>';
    const cors = { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Methods': 'GET,POST,PUT,OPTIONS', 'Access-Control-Allow-Headers': 'content-type', 'Access-Control-Expose-Headers': 'content-disposition' };
    const json = (route, body, status = 200) => route.fulfill({ status, contentType: 'application/json', headers: cors, body: JSON.stringify(body) });
    const capture = source => {
      if (aliases.has(source)) return aliases.get(source);
      if (source.startsWith('/captured/') && assets.has(path.basename(source))) return {src:source,signed_url:`/api/agent/rich-response/asset/${path.basename(source)}?token=fixture`};
      const extension = path.extname(new URL(source, origin).pathname) || '.png';
      const id = createHash('sha256').update(source).digest('hex') + extension;
      const record = { src: `/captured/${id}`, signed_url: `/api/agent/rich-response/asset/${id}?token=fixture` };
      assets.set(id, mediaBytes(source)); aliases.set(source, record); return record;
    };
    await context.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url()), method = request.method();
      if (method === 'OPTIONS') return route.fulfill({ status: 204, headers: cors });
      if (url.hostname === 'rich-media.test') { remoteRequests += 1; return route.fulfill({ status: 200, contentType: 'image/png', body: pixel }); }
      if (url.hostname === 'mermaid-leak.test') { bypassRequests += 1; return route.fulfill({ status: 200, contentType: 'image/png', body: pixel }); }
      if (url.pathname === '/api/agent/rich-response') return route.fulfill({ status: 200, contentType: 'text/html', headers: { 'Content-Security-Policy': hostPolicy }, body: host });
      if (url.pathname === '/api/agent/rich-media') return route.fulfill({ status: 200, contentType: 'text/html', headers: { 'Content-Security-Policy': mediaPolicy }, body: mediaHost });
      if (url.pathname === '/api/agent/rich-response/state') {
        const key = url.searchParams.get('key');
        if (method === 'GET') return json(route, states.get(key) ?? emptyState());
        activeWrites += 1; maxWrites = Math.max(maxWrites, activeWrites);
        const body = request.postDataJSON();
        const rejectSave = failSaves;
        // Controlled slow responses expose overlapping PUTs and stale write races.
        await new Promise(resolve => setTimeout(resolve, apiDelay));
        if (!rejectSave) states.set(key, body);
        activeWrites -= 1;
        return json(route, rejectSave ? { error: 'fixture unavailable' } : body, rejectSave ? 503 : 200);
      }
      if (url.pathname === '/api/agent/rich-response/media') {
        const { src } = request.postDataJSON(); captures.push(src);
        const durable = src.startsWith('/captured/') && assets.has(path.basename(src));
        if (!originalsAvailable && !aliases.has(src) && !durable) return json(route, { error: 'original deleted' }, 404);
        return json(route, capture(src));
      }
      if (url.pathname === '/api/agent/rich-response/remote-media') {
        const { url: src } = request.postDataJSON(); remoteCaptures.push(src); return json(route, capture(src));
      }
      if (url.pathname.startsWith('/api/agent/rich-response/asset/') || url.pathname === '/api/agent/media') {
        const name = url.pathname.endsWith('/media') ? url.searchParams.get('path') : url.pathname;
        const media = assets.get(path.basename(name)) ?? mediaBytes(name);
        const headers = { 'Access-Control-Allow-Origin': '*', ...(url.searchParams.has('download') ? { 'Content-Disposition': `attachment; filename="${path.basename(name)}"` } : {}) };
        return route.fulfill({ status: 200, contentType: media.type, headers, body: media.body });
      }
      if (url.pathname === `/api/agent/artifacts/${artifactId}`) return json(route, { id: artifactId, title: 'Published fixture', html: artifactHtml });
      if (url.pathname === `/api/agent/artifacts/${artifactId}/preview`) {
        const packet = JSON.stringify({ html: documentFor(artifactHtml), nonce: url.searchParams.get('nonce') }).replaceAll('<', '\\u003c');
        return route.fulfill({ status: 200, contentType: 'text/html', headers: { 'Content-Security-Policy': hostPolicy }, body: host.replace('</body>', `<script>window.bruhsCreateWidget(${packet})</script></body>`) });
      }
      if (url.pathname === `/api/agent/artifacts/${artifactId}/diagnostics`) {
        if (method === 'GET') return json(route, diagnostics.get(artifactId) ?? { state: 'published', errors: [] });
        activeReports += 1; maxReports = Math.max(maxReports, activeReports);
        const report = request.postDataJSON(); await new Promise(resolve => setTimeout(resolve, apiDelay));
        diagnostics.set(artifactId, report); activeReports -= 1; return json(route, report);
      }
      if (url.pathname === '/api/agent/rich-response/export') {
        exports.push(request.postDataJSON());
        return route.fulfill({ status: 200, contentType: 'application/zip', headers: { ...cors, 'Content-Disposition': 'attachment; filename="reply.zip"' }, body: Buffer.from('504b0506000000000000000000000000000000000000', 'hex') });
      }
      if (url.pathname === '/api/agent/rich-response/download') return route.fulfill({ status: 200, contentType: 'image/png', headers: { ...cors, 'Content-Disposition': 'attachment; filename="remote.png"' }, body: pixel });
      if (url.pathname.includes('/api/')) return url.pathname.endsWith('/stream') ? route.fulfill({ status: 200, contentType: 'text/event-stream', body: ': fixture\n\n' }) : json(route, {});
      return route.continue();
    });
    await context.addInitScript(() => {
      if (top !== window) return;
      const original = fetch; window.__providerCalls = 0;
      window.fetch = async function(input, init) {
        const url = typeof input === 'string' ? input : (input.url ?? input.href), method = init?.method ?? input.method ?? 'GET';
        if (new URL(url, location.href).pathname === '/api/agent' && method === 'POST') {
          window.__providerCalls += 1;
          return new Response(new ReadableStream({ start(controller) { window.__richStream = controller; } }), { headers: { 'Content-Type': 'text/event-stream' } });
        }
        return original.apply(this, arguments);
      };
    });
    const page = await context.newPage(), pageErrors = [];
    page.on('pageerror', error => { if (!error.message.includes('fixture artifact runtime failure')) pageErrors.push(error.message); });
    if (process.env.BRUHS_BROWSER_DEBUG) page.on('console', message => { if (message.type() === 'error') console.error(message.text()); });
    await page.goto(`${origin}/#/chat`);
    await page.evaluate(() => new Promise(resolve => { const image = new Image(); image.onload = image.onerror = () => { image.remove(); resolve(); }; image.src = 'https://mermaid-leak.test/probe'; document.body.append(image); }));
    check(bypassRequests === 0, 'Main CSP blocks remote image nodes outside the consent component');
    await page.locator('#chat-input').fill('Show the browser response fixture');
    await page.getByRole('button', { name: 'Send message', exact: true }).click(); await page.waitForFunction(() => !!window.__richStream);
    const emit = value => page.evaluate(value => window.__richStream.enqueue(new TextEncoder().encode(`data: ${JSON.stringify({ envelope: 'query', ...value })}\n\n`)), value);
    await emit({ type: 'text', text: `Before\n\n\x60\x60\x60bruhs-html Counter\n${probe.replaceAll('__ORIGIN__', origin)}\n` });
    await page.getByText('Before', { exact: false }).first().waitFor();
    check(await page.locator('[data-response-interactive]').count() === 0, 'Partial rich fences stay inert while streaming');
    await emit({ type: 'text', text: '\x60\x60\x60\n\n\x60\x60\x60bruhs-media\n{"kind":"image","src":"https://rich-media.test/probe.png","title":"Remote image"}\n\x60\x60\x60\n\n![Local image](output/chart.png)\n\n| Picture | Details |\n|---|---|\n| **![Table image](output/table.png)** | Kept in a table |\n\n- ![List image](output/list.png)\n\nTail' });
    const rich = page.locator('[data-response-interactive]').filter({ hasText: 'Counter' }).first();
    const liveWidget = rich.frameLocator('iframe').frameLocator('iframe');
    await rich.getByText('Loading interactive response…').waitFor({ state: 'hidden' });
    await liveWidget.locator('#state').evaluate(() => window.bruhs.ready);
    await liveWidget.locator('#state').fill('kept across streaming');
    await liveWidget.locator('#secret').fill('never persist this secret'); await liveWidget.locator('#check').check();
    await liveWidget.locator('#counter').evaluate(button => button.click());
    await liveWidget.locator('#counter').filter({ hasText: '1' }).waitFor();
    const key = await rich.getAttribute('data-widget-state-key');
    const outer = await (await rich.locator('iframe').elementHandle()).contentFrame(), liveFrame = outer.childFrames()[0];
    const identity = await liveFrame.evaluate(() => window.identity);
    check(remoteRequests === 0, 'External images make no request before Load');
    const remote = page.locator('[data-response-media="image"]').filter({ hasText: 'Remote image' });
    await remote.getByRole('button', { name: 'Load image' }).click();
    const remoteImage = remote.frameLocator('iframe').locator('img'); await remoteImage.waitFor(); await remoteImage.evaluate(image => image.decode());
    check(await remoteImage.evaluate(image => image.complete && image.naturalWidth === 1), 'Consent loads and decodes external media inline');
    const tableImage = page.locator('table td [data-response-media="image"] img'); await tableImage.waitFor();
    check(await page.locator('table').count() === 1 && await page.locator('li [data-response-media="image"] img').count() === 1, 'Nested media preserves table and list structure');
    const tableNode = await tableImage.elementHandle();
    await emit({ type: 'text', text: '\n\n\x60\x60\x60bruhs-media\n{"kind":"audio","src":"output/silent.wav","title":"Local audio","transcript":"An accessible audio transcript."}\n\x60\x60\x60\n\n' });
    const audio = page.locator('[data-response-media="audio"] audio'); await audio.waitFor();
    check(await audio.evaluate(media => media.paused && media.controls && media.preload === 'none'), 'Audio has controls without autoplay or eager playback');
    await audio.evaluate(async media => { media.muted = true; await media.play(); media.pause(); });
    check(await audio.evaluate(media => media.duration === 1 && media.readyState >= 2), 'Audio decodes and plays inline');
    if (video.status === 0) {
      await emit({ type: 'text', text: '\n\n\x60\x60\x60bruhs-media\n{"kind":"video","src":"output/clip.webm","poster":"output/poster.png","tracks":[{"src":"output/captions.vtt","label":"English","language":"en"}],"title":"Captioned local video","transcript":"Local video transcript."}\n\x60\x60\x60\n\n' });
      const player = page.locator('[data-response-media="video"] video'); await player.waitFor();
      check(await player.evaluate(media => media.paused && media.controls && media.preload === 'none' && !!media.poster && media.querySelector('track')?.label === 'English'), 'Video retains poster and caption metadata before playback');
      await player.evaluate(async media => { media.muted = true; await media.play(); media.pause(); });
      check(await player.evaluate(media => media.videoWidth === 16 && media.readyState >= 2), 'Captioned video decodes and plays inline');
      await emit({ type: 'text', text: '\n\n\x60\x60\x60bruhs-media\n{"kind":"video","src":"https://nocors-media.test/clip.webm","poster":"output/poster.png","tracks":[{"src":"https://nocors-media.test/captions.vtt","label":"Mixed captions","language":"en"}],"title":"Mixed video"}\n\x60\x60\x60\n\n' });
      const mixed = page.locator('[data-response-media="video"]').filter({ hasText: 'Mixed video' });
      await mixed.getByRole('button', { name: 'Load video' }).waitFor(); check(remoteCaptures.length === 0, 'Mixed metadata makes no external proxy request before consent');
      await mixed.getByRole('button', { name: 'Load video' }).click();
      const mixedVideo = mixed.frameLocator('iframe').locator('video'); await mixedVideo.waitFor();
      await mixedVideo.evaluate(async media => { media.muted = true; await media.play(); media.pause(); });
      check(await mixedVideo.evaluate(media => media.videoWidth === 16 && media.querySelector('track')?.label === 'Mixed captions'), 'Explicit Load proxies mixed video and VTT into CORS-safe durable assets');
    }
    check(captures.includes('output/silent.wav') && (!video.status ? captures.includes('output/clip.webm') && captures.includes('output/captions.vtt') : true), 'Local assets are captured even with preload none');
    await emit({ type: 'text', text: 'Streamed tail remains responsive' }); await page.getByText('Streamed tail remains responsive', { exact: true }).waitFor();
    check(await liveFrame.evaluate(() => window.identity) === identity, 'Streaming keeps the same interactive iframe');
    check(await liveWidget.locator('#state').inputValue() === 'kept across streaming' && await liveWidget.locator('#counter').textContent() === '1', 'Streaming preserves form and custom widget state');
    check(await tableNode.evaluate(node => node.isConnected), 'Streaming preserves nested media DOM identity');
    check(await remoteImage.count() === 1 && remoteRequests === 1, 'Streaming preserves external media consent');
    await liveFrame.evaluate(() => { const spacer = document.createElement('div'); spacer.id = 'spacer'; spacer.style.height = '600px'; document.body.append(spacer); });
    await page.waitForFunction(() => [...document.querySelectorAll('[data-response-interactive]')][0].querySelector('iframe').getBoundingClientRect().height >= 600);
    check(await rich.locator('iframe').evaluate(frame => frame.getBoundingClientRect().height >= 600), 'Widget content automatically resizes its chat frame');
    await liveFrame.evaluate(() => document.getElementById('spacer').remove());
    await page.evaluate(() => document.documentElement.style.setProperty('--color-persimmon-400', 'rgb(1, 2, 3)'));
    try {
      await liveFrame.waitForFunction(() => document.documentElement.style.getPropertyValue('--bruhs-accent') === 'rgb(1, 2, 3)');
    } catch (error) {
      console.error('Theme diagnostics', JSON.stringify({ main: await page.evaluate(() => document.documentElement.getAttribute('style')), child: await liveFrame.evaluate(() => document.documentElement.getAttribute('style')), errors: pageErrors }));
      throw error;
    }
    check(await liveWidget.locator('#state').inputValue() === 'kept across streaming', 'Theme updates preserve user-entered state');
    const nonce = await rich.locator('iframe').evaluate(frame => JSON.parse(decodeURIComponent(new URL(frame.src).hash.slice(1))).nonce);
    await page.evaluate(nonce => postMessage({ bruhs: 1, nonce, type: 'action', action: { label: 'Forged', message: 'Do not insert this' } }, '*'), nonce);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
    check(await page.locator('[data-widget-action-review]').count() === 0, 'Spoofed packets from another window are ignored');
    await liveWidget.locator('#submit').evaluate(button => button.click()); await rich.locator('[data-widget-action-review]').waitFor();
    check(await page.locator('#chat-input').inputValue() === '' && await page.evaluate(() => __providerCalls) === 1, 'Widget actions remain reviewable and never send messages automatically');
    await rich.getByRole('button', { name: 'Insert into composer' }).click();
    check((await page.locator('#chat-input').inputValue()).includes('A reviewed widget draft') && await page.evaluate(() => __providerCalls) === 1, 'Reviewed widget action inserts a draft without sending');
    await page.locator('#chat-input').fill('');
    await rich.getByRole('button', { name: 'Source', exact: true }).click(); await rich.locator('pre').first().waitFor(); await rich.getByRole('button', { name: 'Preview', exact: true }).click();
    check(await liveWidget.locator('#state').inputValue() === 'kept across streaming', 'Source toggle preserves current widget state');
    await eventually(() => states.get(key)?.fields['id:state']?.value === 'kept across streaming', 'Widget state never reached its durable API');
    check(maxWrites === 1 && !JSON.stringify(states.get(key)).includes('never persist this secret'), 'State writes serialize and exclude password values');
    await emit({ type: 'tool_call', id: 'publish-1', tool: 'mcp__bruhs_artifacts__publish_artifact', input: { path: 'fixture.html' } });
    await emit({ type: 'tool_result', tool_use_id: 'publish-1', content: `\x60\x60\x60bruhs-artifact\n${JSON.stringify({ id: artifactId, title: 'Published fixture' })}\n\x60\x60\x60` });
    const published = page.locator(`[data-response-artifact="${artifactId}"]`); await published.locator('iframe').waitFor();
    await published.getByText('fixture artifact runtime failure', { exact: false }).first().waitFor();
    await eventually(() => diagnostics.get(artifactId)?.state === 'error', 'Artifact error diagnostics never reached the API');
    check(maxReports === 1 && diagnostics.get(artifactId).errors.some(error => error.includes('fixture artifact runtime failure')), 'Published tools preview automatically and report ordered runtime diagnostics');
    const immutableBindings=Object.fromEntries([...aliases.entries()].filter(([source])=>!source.startsWith('http')).map(([source,asset])=>[source,asset.src]));
    const audioBeforeBinding=await audio.elementHandle();
    await emit({type:'response_media',bindings:immutableBindings});
    await emit({ type: 'result', is_error: false }); await page.evaluate(() => __richStream.close()); await page.getByRole('button', { name: 'Send message', exact: true }).waitFor();
    check(await audioBeforeBinding.evaluate(node=>node.isConnected) && await tableNode.evaluate(node=>node.isConnected), 'Immutable media bindings preserve mounted top-level and nested players');
    check(await liveWidget.locator('#state').inputValue() === 'kept across streaming', 'Settling the response keeps live widget state');
    await rich.getByRole('button', { name: 'Fullscreen interactive response' }).click(); await page.waitForFunction(() => !!document.fullscreenElement);
    check(await page.evaluate(() => !!document.fullscreenElement), 'Widget fullscreen opens from its chat control'); await page.evaluate(() => document.exitFullscreen());
    const localImage = page.locator('[data-response-media="image"]').filter({ hasText: 'Local image' }).first();
    await localImage.getByRole('button', { name: 'Zoom image' }).click(); const zoom = page.getByRole('dialog', { name: 'Image zoom' }); await zoom.waitFor();
    await zoom.getByRole('slider', { name: 'Image zoom' }).press('End');
    check(await zoom.locator('img').count() === 1 && await zoom.getByRole('slider', { name: 'Image zoom' }).inputValue() === '4', 'Image zoom opens an accessible adjustable viewer'); await zoom.getByRole('button', { name: 'Close image' }).click();
    const htmlDownload = page.waitForEvent('download'); await rich.getByRole('button', { name: 'Download HTML' }).click(); const downloadedHtml = await htmlDownload;
    const offline = await readFile(await downloadedHtml.path(), 'utf8');
    check(downloadedHtml.suggestedFilename() === 'widget.html' && offline.includes('bruhs') && !offline.includes('kept across streaming'), 'Standalone HTML download keeps typed widget values private by default');
    const remoteDownload = page.waitForEvent('download'); await remote.getByRole('button', { name: 'Download', exact: true }).click(); check((await remoteDownload).suggestedFilename() === 'remote.png', 'Explicit remote download uses the bounded POST endpoint');
    const exporter = page.locator('[data-response-export]').first(); await exporter.getByRole('button', { name: 'Export reply' }).click();
    const zipDownload = page.waitForEvent('download'); await exporter.getByRole('button', { name: 'Download ZIP' }).click(); check((await zipDownload).suggestedFilename() === 'reply.zip' && !exports.at(-1).include_remote && !exports.at(-1).include_state, 'Reply export defaults to local assets and excludes private widget values');
    await exporter.getByLabel('Include external media').check(); await exporter.getByLabel('Include saved widget values').check(); const explicitZip = page.waitForEvent('download'); await exporter.getByRole('button', { name: 'Download ZIP' }).click(); await explicitZip;
    check(exports.at(-1).include_remote && exports.at(-1).include_state && !!exports.at(-1).scope, 'Bundle options explicitly opt into remote media and saved values');
    failSaves=true; await liveWidget.locator('#state').fill('unsaved export snapshot');
    await page.waitForFunction(()=>Object.keys(localStorage).some(key=>key.startsWith('bruhs.widget.pending.') && localStorage.getItem(key).includes('unsaved export snapshot')));
    const pendingExport=page.waitForEvent('download'); await exporter.getByRole('button',{name:'Download ZIP'}).click(); await pendingExport;
    check(Object.values(exports.at(-1).widget_states).some(state=>state.fields['id:state']?.value==='unsaved export snapshot'),'State export includes current values without waiting for pending PUTs');
    check(exports.at(-1).state_keys.includes(key) && exports.at(-1).markdown.includes(immutableBindings['output/silent.wav']), 'Export uses ordered response keys and immutable historical media paths');
    failSaves = true; await liveWidget.locator('#state').fill('recovered final edit');
    await page.waitForFunction(() => Object.keys(localStorage).some(key => key.startsWith('bruhs.widget.pending.') && localStorage.getItem(key).includes('recovered final edit')));
    failSaves = false; originalsAvailable = false; await page.reload(); await liveWidget.locator('#state').waitFor();
    check(await liveWidget.locator('#state').inputValue() === 'recovered final edit', 'Pending local recovery survives hard reload before a successful server save');
    check(await liveWidget.locator('#check').isChecked() && await liveWidget.locator('#secret').inputValue() === '', 'Reload restores eligible form values without restoring secrets');
    await liveWidget.locator('#counter').filter({ hasText: '1' }).waitFor(); check(true, 'Custom widget state replays after page reload');
    await remote.getByRole('button', { name: 'Load image' }).waitFor(); check(remoteRequests === 1 && await remote.locator('iframe').count() === 0, 'Replayed external media needs fresh consent');
    await tableImage.waitFor(); check(await tableImage.evaluate(image => image.complete && image.naturalWidth === 1), 'Captured media survives removal of its original project file');
    await rich.scrollIntoViewIfNeeded();
    await liveWidget.locator('#state').focus();
    await liveWidget.locator('#state').scrollIntoViewIfNeeded();
    await liveWidget.locator('#state').evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    if (evidence) await page.screenshot({ path: path.join(evidence, 'rich-response-desktop.png') });
    await page.setViewportSize({ width: 768, height: 1024 });
    check(await rich.evaluate(element => element.getBoundingClientRect().right <= innerWidth), 'Rich controls remain within the tablet viewport');
    await page.setViewportSize({ width: 390, height: 844 });
    const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0.z-30:visible').first(); if (await scrim.count()) { const bounds = await scrim.boundingBox(); await scrim.click({ position: { x: bounds.width - 1, y: 1 } }); }
    check(await rich.evaluate(element => element.getBoundingClientRect().right <= innerWidth), 'Rich controls remain within the touch viewport');
    const cdp = await context.newCDPSession(page);
    await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 });
    await cdp.send('Network.enable');
    await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 400, downloadThroughput: 50_000, uploadThroughput: 50_000, connectionType: 'cellular3g' });
    apiDelay = 400;
    const checkbox = liveWidget.locator('#check');
    await checkbox.evaluate(element => element.addEventListener('pointerdown', event => { element.dataset.pointerType = event.pointerType; }));
    const tapCheckbox = async () => {
      await checkbox.scrollIntoViewIfNeeded();
      const bounds = await checkbox.boundingBox();
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 }] });
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    };
    const slowSaveStarted = Date.now();
    await tapCheckbox();
    await eventually(() => states.get(key)?.fields['id:check']?.checked === false, 'Touch changes did not reach the delayed state API');
    check(await checkbox.getAttribute('data-pointer-type') === 'touch' && !(await checkbox.isChecked()), 'Actual touch input operates the sandboxed widget form');
    check(Date.now() - slowSaveStarted >= 400 && maxWrites === 1, 'Widget state survives 3G emulation and delayed serialized saves');
    await tapCheckbox();
    await eventually(() => states.get(key)?.fields['id:check']?.checked === true, 'Restored touch selection did not persist');
    if (evidence) await page.screenshot({ path: path.join(evidence, 'rich-response-touch.png') });
    apiDelay = 25;
    await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1, connectionType: 'none' });
    const beforeReload = await rich.locator('iframe').elementHandle(); await rich.getByRole('button', { name: 'Reload interactive response' }).click(); await page.waitForFunction(frame => !frame.isConnected, beforeReload); await liveWidget.locator('#state').waitFor(); await liveWidget.locator('#state').evaluate(() => window.bruhs.ready);
    check(await liveWidget.locator('#state').inputValue() === 'recovered final edit', 'Reload keeps saved values until explicit Reset');
    const beforeReset = await rich.locator('iframe').elementHandle(); await rich.getByRole('button', { name: 'Reset saved widget values' }).click(); await page.waitForFunction(frame => !frame.isConnected, beforeReset); await liveWidget.locator('#state').waitFor(); await liveWidget.locator('#state').evaluate(() => window.bruhs.ready);
    check(await liveWidget.locator('#state').inputValue() === 'initial', 'Explicit Reset returns the widget to its original values');
    check(pageErrors.length === 0, `Application has no uncaught errors: ${pageErrors.join('; ')}`);
    await context.close();
  }
  const result = JSON.stringify({ passed: passed.length, checks: passed, skipped: video.status === 0 ? [] : ['Video fixtures require ffmpeg'] }, null, 2);
  if (evidence) await writeFile(path.join(evidence, 'result.json'), `${result}\n`);
  console.log(result);
} finally {
  await browser.close(); await new Promise(resolve => server.close(resolve));
}
