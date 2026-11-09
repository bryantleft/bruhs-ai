// Page-lifetime browser glue the WASM app calls through `window`: the
// xterm session host (`window.bruhsTerminal`) and the lazy KaTeX/Mermaid
// loaders (`window.ensureMath`, `window.ensureMermaid`). Loaded as a file,
// not inline, so the Content-Security-Policy can forbid inline scripts.
'use strict';

// Sessions live here for the life of the page: xterm instance, its
// container element, and the WebSocket to the server PTY. Rust attaches
// the active session's container to whatever mount is on screen.
window.bruhsTerminal = (() => {
  const sessions = new Map();
  let statusCb = null;
  let titleCb = null;
  // bruhs tones: longan canvas, rambutan text, persimmon cursor, fruit ANSI.
  const theme = {
    background: '#141210', foreground: '#d9cdb9', cursor: '#f08a3c', cursorAccent: '#141210',
    selectionBackground: '#3b3328', selectionForeground: '#f2eadc',
    black: '#2a241d', red: '#e57373', green: '#8fcf8f', yellow: '#e9c46a', blue: '#7fa7ff',
    magenta: '#c99be0', cyan: '#7fd1d1', white: '#d9cdb9',
    brightBlack: '#8a7b68', brightRed: '#ff9a9a', brightGreen: '#a8e6a8', brightYellow: '#f5d78a',
    brightBlue: '#a7cbff', brightMagenta: '#dbb9ec', brightCyan: '#a0e6e6', brightWhite: '#f6efe3',
  };
  const ready = () => typeof window.Terminal === 'function' && !!window.FitAddon;
  const setStatus = (id, status) => {
    const s = sessions.get(id);
    if (!s) return;
    s.status = status;
    if (statusCb) { try { statusCb(id, status); } catch (_) {} }
  };
  const encoder = new TextEncoder();
  const api = {
    ready,
    onStatus(cb) { statusCb = cb; },
    onTitle(cb) { titleCb = cb; },
    has(id) { return sessions.has(id); },
    status(id) { return sessions.get(id)?.status ?? 'missing'; },
    selection(id) {
      const s = sessions.get(id);
      if (!s) return '';
      try { return s.term.getSelection() || ''; } catch (_) { return ''; }
    },
    create(id, wsUrl) {
      if (!ready() || sessions.has(id)) return false;
      const container = document.createElement('div');
      container.style.height = '100%';
      container.style.width = '100%';
      const term = new window.Terminal({
        theme,
        fontFamily: '"MonaspaceNeonVariable", "Monaspace Neon Var", "Monaspace Neon", ui-monospace, SFMono-Regular, Menlo, monospace',
        fontSize: 12,
        lineHeight: 1.25,
        cursorBlink: true,
        scrollback: 5000,
        allowProposedApi: true,
      });
      const fit = new window.FitAddon.FitAddon();
      term.loadAddon(fit);
      const s = { term, fit, container, ws: null, status: 'connecting', opened: false };
      sessions.set(id, s);
      const ws = new WebSocket(wsUrl);
      ws.binaryType = 'arraybuffer';
      s.ws = ws;
      ws.onopen = () => { setStatus(id, 'live'); api.fit(id); };
      ws.onmessage = (ev) => {
        term.write(typeof ev.data === 'string' ? ev.data : new Uint8Array(ev.data));
      };
      ws.onclose = () => {
        if (s.status !== 'failed') setStatus(id, 'ended');
        term.write('\r\n\x1b[2m[session ended]\x1b[0m\r\n');
      };
      ws.onerror = () => setStatus(id, 'failed');
      term.onData((data) => {
        if (ws.readyState === WebSocket.OPEN) ws.send(encoder.encode(data));
      });
      term.onResize(({ cols, rows }) => {
        if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      });
      // OSC 0/2 titles from the shell name the tab.
      term.onTitleChange((title) => {
        if (titleCb) { try { titleCb(id, title); } catch (_) {} }
      });
      return true;
    },
    attach(id, el) {
      const s = sessions.get(id);
      if (!s || !el) return;
      if (s.container.parentElement !== el) el.replaceChildren(s.container);
      if (!s.opened) { s.term.open(s.container); s.opened = true; }
      api.fit(id);
      s.term.focus();
    },
    detach(id) {
      const s = sessions.get(id);
      if (s && s.container.parentElement) s.container.remove();
    },
    fit(id) {
      const s = sessions.get(id);
      if (!s || !s.opened || !s.container.isConnected) return;
      try { s.fit.fit(); } catch (_) {}
    },
    focus(id) { sessions.get(id)?.term.focus(); },
    close(id) {
      const s = sessions.get(id);
      if (!s) return;
      try { s.ws.close(); } catch (_) {}
      try { s.term.dispose(); } catch (_) {}
      s.container.remove();
      sessions.delete(id);
    },
  };
  // Refit whatever is attached when the window resizes.
  window.addEventListener('resize', () => { for (const id of sessions.keys()) api.fit(id); });
  return api;
})();

// Lazy loaders for highlight.js, xterm, math, and diagrams. Each is
// idempotent: one shared promise, scripts injected once. render_pass.rs and
// terminal_panel.rs call them the first time a message or terminal needs them.
(() => {
  const loadScript = (src) =>
    new Promise((resolve, reject) => {
      const s = document.createElement('script');
      s.src = src;
      s.onload = resolve;
      s.onerror = reject;
      document.head.appendChild(s);
    });
  const loadStyle = (href) =>
    new Promise((resolve, reject) => {
      const link = document.createElement('link');
      link.rel = 'stylesheet';
      link.href = href;
      link.onload = resolve;
      link.onerror = reject;
      document.head.appendChild(link);
    });
  const once = (load) => {
    let promise = null;
    return () => {
      if (!promise) promise = load();
      return promise;
    };
  };
  window.ensureHighlight = once(() =>
    Promise.all([
      loadStyle('/vendor/highlight-11.10.0/github-dark.min.css'),
      loadScript('/vendor/highlight-11.10.0/highlight.min.js'),
    ]),
  );
  window.ensureXterm = once(() =>
    Promise.all([
      loadStyle('/vendor/xterm-5.5.0/xterm.css'),
      loadScript('/vendor/xterm-5.5.0/xterm.js').then(() =>
        loadScript('/vendor/addon-fit-0.10.0/addon-fit.js'),
      ),
    ]),
  );
  window.ensureMath = once(async () => {
    await loadScript('/vendor/katex-0.16.11/katex.min.js');
    await loadScript('/vendor/katex-0.16.11/contrib/auto-render.min.js');
  });
  window.ensureMermaid = once(async () => {
    // Mermaid dark theme on bruhs longan-950 canvas tokens.
    const mermaid = (await import('/vendor/mermaid-11.17.2/mermaid.esm.min.mjs')).default;
    mermaid.initialize({
      startOnLoad: false,
      // Diagram text comes from assistant replies: no HTML labels or
      // click handlers (Mermaid's default, stated so it cannot drift).
      securityLevel: 'strict',
      theme: 'dark',
      themeVariables: {
        background: '#141210',
        primaryColor: '#2f2922',
        primaryTextColor: '#dfcfc3',
        primaryBorderColor: '#594a36',
        lineColor: '#c3afa1',
        fontFamily: 'MonaspaceNeonVariable, Monaspace Neon, ui-monospace, monospace',
      },
    });
    window.mermaid = mermaid;
  });
  // Both, for callers that want everything.
  window.ensureMathTools = () => Promise.all([window.ensureMath(), window.ensureMermaid()]);
})();
