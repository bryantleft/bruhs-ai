// Headless Chrome CDP runner for landing-page screenshots.
// Usage: node shots.mjs <origin> <seed.json> <outDir> <cdpPort> [shotName ...]
// Seeds IndexedDB `bruhs` (v11) before the app ever opens it, pins the clock to
// Wed 2026-11-04 11:20 America/Chicago, then captures 1440x900 @2x PNGs.
import { readFileSync, writeFileSync } from 'node:fs';

const [origin, seedPath, outDir, cdpPort = '9351', ...only] = process.argv.slice(2);
const seed = JSON.parse(readFileSync(seedPath, 'utf8'));
const FIXED_NOW = Date.parse('2026-11-04T11:20:00-06:00');
const W = 1440, H = 900;

const list = await (await fetch(`http://127.0.0.1:${cdpPort}/json/list`)).json();
const page = list.find(t => t.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => (ws.onopen = r));
let seq = 0;
const pending = new Map();
const consoleLines = [];
ws.onmessage = ({ data }) => {
  const msg = JSON.parse(data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  else if (msg.method === 'Runtime.consoleAPICalled') {
    const text = msg.params.args.map(a => a.value ?? a.description ?? '').join(' ');
    if (msg.params.type === 'error' || msg.params.type === 'warning') consoleLines.push(`[${msg.params.type}] ${text}`.slice(0, 300));
  } else if (msg.method === 'Runtime.exceptionThrown') {
    consoleLines.push(`[exception] ${JSON.stringify(msg.params.exceptionDetails).slice(0, 300)}`);
  }
};
const send = (method, params = {}) => new Promise((res, rej) => {
  const id = ++seq; pending.set(id, m => (m.error ? rej(new Error(`${method}: ${m.error.message}`)) : res(m.result)));
  ws.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression) => {
  const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 500));
  return r.result?.value;
};
const sleep = ms => new Promise(r => setTimeout(r, ms));
const key = async (k, code, vk, modifiers = 0) => {
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code, windowsVirtualKeyCode: vk, modifiers });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk, modifiers });
};
const typeText = async (s) => { for (const ch of s) await send('Input.insertText', { text: ch }); };
const clickText = (selector, text) => evaluate(`(() => {
  const el = [...document.querySelectorAll(${JSON.stringify(selector)})].find(e => e.textContent.trim() === ${JSON.stringify(text)});
  if (!el) return false; el.click(); return true; })()`);

await send('Page.enable');
await send('Runtime.enable');
await send('Emulation.setTimezoneOverride', { timezoneId: 'America/Chicago' });
await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 2, mobile: false });
await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
// Pin the clock before any page script runs (the offset keeps time flowing).
await send('Page.addScriptToEvaluateOnNewDocument', { source: `(() => {
  const RealDate = Date, off = ${FIXED_NOW} - RealDate.now();
  class PinnedDate extends RealDate {
    constructor(...a) { if (a.length === 0) super(RealDate.now() + off); else super(...a); }
    static now() { return RealDate.now() + off; }
  }
  globalThis.Date = PinnedDate;
})();` });

const navigate = async (url, wait = 3000) => { await send('Page.navigate', { url }); await sleep(wait); };

// ── Seed: create the v11 schema ourselves, then fill it. ─────────────────────
await navigate(`${origin}/__seed-blank`, 800);
const seedResult = await evaluate(`(async () => {
  const seed = ${JSON.stringify(seed)};
  await new Promise((res) => { const r = indexedDB.deleteDatabase('bruhs'); r.onsuccess = r.onerror = r.onblocked = () => res(); });
  localStorage.clear();
  localStorage.setItem('bruhs.workspaceMode', 'code');
  localStorage.setItem('bruhs.model', 'claude-opus-4-8');
  const stores = [['metadata','key'],['threads','id'],['messages','thread_id'],['work_items','id'],['projects','id'],['events','id'],['calendars','id'],['meetings','id'],['meeting_notes','id'],['meeting_generations','id'],['transcript_segments','id'],['people','id'],['captures','id'],['knowledge_documents','id'],['source_clips','id'],['knowledge_attachments','id'],['inbox_conversations','id'],['relationships','person_id']];
  const db = await new Promise((res, rej) => {
    const r = indexedDB.open('bruhs', 11);
    r.onupgradeneeded = () => {
      const d = r.result;
      for (const [name, key] of stores) {
        const s = d.createObjectStore(name, { keyPath: key });
        if (name === 'transcript_segments') s.createIndex('by_meeting', 'meeting_id');
      }
    };
    r.onsuccess = () => res(r.result); r.onerror = () => rej(r.error);
  });
  const put = (store, rows) => new Promise((res, rej) => {
    const tx = db.transaction([store], 'readwrite');
    for (const row of rows) tx.objectStore(store).put(row);
    tx.oncomplete = res; tx.onabort = tx.onerror = () => rej(tx.error);
  });
  await put('projects', seed.projects);
  await put('work_items', seed.work_items);
  await put('calendars', seed.calendars);
  await put('events', seed.events);
  await put('people', seed.people);
  await put('relationships', seed.relationships);
  await put('meetings', seed.meetings);
  await put('transcript_segments', seed.segments);
  await put('knowledge_documents', seed.knowledge_documents);
  await put('source_clips', seed.source_clips);
  await put('threads', seed.threads);
  await put('messages', seed.messages);
  await put('metadata', [{ key: 'bruhs.lifeContext', revision: 1, value: seed.life }]);
  db.close();
  return 'seeded';
})()`);
console.error('seed:', seedResult);

const settle = () => evaluate('document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))');
const capture = async (name) => {
  await settle();
  const r = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  const file = `${outDir}/${name}.png`;
  writeFileSync(file, Buffer.from(r.data, 'base64'));
  return file;
};
// Visible text that would betray a fixture: errors, offline, empty states.
const probe = () => evaluate(`(() => {
  const txt = document.body.innerText;
  const bad = /offline|reconnect|could not|couldn't|failed|unavailable|error|not running|no (threads|tasks|events|meetings|people|documents)|nothing (here|yet)/i;
  const lines = txt.split('\\n').map(s => s.trim()).filter(Boolean);
  return { flagged: lines.filter(l => bad.test(l)).slice(0, 12), fonts: [...document.fonts].filter(f => f.status === 'loaded').map(f => f.family).filter((v, i, a) => a.indexOf(v) === i) };
})()`);

const expandWork = async () => {
  await evaluate(`(() => { const b = [...document.querySelectorAll('button')].find(e => /^Worked for/.test(e.textContent.trim())); if (b) b.click(); return !!b; })()`);
  await sleep(700);
  if (process.env.EXPAND_GROUPS !== '0') {
    const idx = (process.env.EXPAND_GROUPS ?? 'all') === 'all' ? [0, 1, 2, 3] : process.env.EXPAND_GROUPS.split(',').map(Number);
    console.error('groups:', await evaluate(`(() => { const bs = [...document.querySelectorAll('button')].filter(e => /^(Read|Edited|Ran|Searched) /.test(e.textContent.trim())); for (const i of ${JSON.stringify(idx)}) bs[i] && bs[i].click(); return bs.map(b => b.textContent.trim()); })()`));
    await sleep(700);
  }
  if (process.env.EXPAND_LAST_BASH) {
    await evaluate(`(() => { const bs = [...document.querySelectorAll('button')].filter(e => /^Ran cargo/.test(e.textContent.trim())); const b = bs[bs.length - 1]; if (b) b.click(); return bs.length; })()`);
    await sleep(700);
  }
};
const scrollMain = async (px) => {
  await evaluate(`(() => { const els = [...document.querySelectorAll('main *, section *')].filter(e => e.scrollHeight > e.clientHeight + 20 && /(auto|scroll)/.test(getComputedStyle(e).overflowY) && e.getBoundingClientRect().left > 300); const el = els.sort((a, b) => b.clientHeight - a.clientHeight)[0]; if (el) el.scrollTop = ${px}; return !!el; })()`);
  await sleep(400);
};
let n = 0;
const go = async (route, wait = 3500) => navigate(`${origin}/?r=${++n}${route}`, wait);
const shots = {
  'hero-agents': async () => { await go('#thread/th-auth', 4500); await expandWork(); },
  'hero-palette': async () => {
    await go('#thread/th-auth', 4500);
    await expandWork();
    await key('k', 'KeyK', 75, 4); // Meta
    await sleep(500);
    const q = process.env.PALETTE_QUERY ?? 'plan'; if (q) { await typeText(q); await sleep(400); }
  },
  'hero-palette-empty': async () => {
    await go('#thread/th-auth', 4500);
    await expandWork();
    await key('k', 'KeyK', 75, 4);
    await sleep(500);
  },
  tasks: async () => { await go('#/tasks'); },
  today: async () => { await go('#/meetings'); await evaluate("location.hash = '#/today'"); await sleep(1500); },
  'tasks-board': async () => { await go('#/tasks'); await clickText('button', 'Board'); await sleep(800); },
  calendar: async () => { await go('#/calendar'); await clickText('button', 'Week'); await sleep(800); },
  meetings: async () => {
    await go('#/meetings/mtg-kickoff', 4500);
    // Scroll the detail pane so the summary sits under the transcript's tail.
    await evaluate(`(() => { const h = [...document.querySelectorAll('div')].find(e => e.textContent.trim() === 'Summary'); if (!h) return false; h.scrollIntoView({ block: 'start' }); let p = h.parentElement; while (p && p.scrollHeight <= p.clientHeight) p = p.parentElement; if (p) p.scrollTop -= Number(process.env.MEETING_OFFSET || 260); return true; })()`.replace('process.env.MEETING_OFFSET || 260', String(process.env.MEETING_OFFSET || 260)));
    await sleep(500);
  },
  'meetings-list': async () => { await go('#/meetings'); },
  people: async () => {
    await go('#/people');
    await evaluate(`(() => { const b = [...document.querySelectorAll('[aria-expanded]')].find(e => e.textContent.includes('Maya Chen')); if (b) b.click(); return !!b; })()`);
    await sleep(1000);
  },
  notes: async () => { await go('#/knowledge'); },
  life: async () => { await go('#/life'); },
  'life-budgets': async () => {
    await go('#/life');
    console.error('life scroll:', await evaluate(`(() => { const el = document.querySelector('section[aria-labelledby="life-title"] > div.overflow-y-auto'); if (!el) return null; el.scrollTop = ${Number(process.env.LIFE_SCROLL || 640)}; return [el.scrollTop, el.scrollHeight, el.clientHeight]; })()`));
    await sleep(400);
  },
  'notes-doc': async () => { await go('#/knowledge/documents/doc-pilot'); },
  'life-health': async () => { await go('#/life'); await clickText('button[role="tab"]', 'Health'); await sleep(600); },
};

const DEFAULT = ['hero-agents', 'hero-palette', 'hero-palette-empty', 'tasks', 'today', 'calendar', 'meetings', 'people', 'notes', 'life', 'life-health'];
const wanted = only.length ? only : DEFAULT;
const report = [];
for (const [name, run] of Object.entries(shots)) {
  if (!wanted.includes(name)) continue;
  consoleLines.length = 0;
  await run();
  const file = await capture(name);
  report.push({ name, file, url: await evaluate('location.href'), probe: await probe(), console: consoleLines.slice(0, 8) });
}
ws.close();
writeFileSync(process.env.REPORT ?? "/tmp/bruhs-shots/report.json", JSON.stringify(report, null, 1)); for (const r of report) console.log(r.name, r.url, JSON.stringify(r.probe.flagged), JSON.stringify(r.console));
