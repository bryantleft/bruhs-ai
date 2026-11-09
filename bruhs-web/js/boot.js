// Starts the WASM app. Trunk's default loader is an inline <script>, which
// the Content-Security-Policy in index.html forbids; `pattern_script` in
// Trunk.toml emits `<script type="module" src="/js/boot.js" data-js=… data-wasm=…>`
// instead, and this module does what the inline loader did.
const tag = document.querySelector('script[data-bruhs-boot]');
const js = tag?.dataset.js;
const wasm = tag?.dataset.wasm;
if (!js || !wasm) {
  throw new Error('bruhs boot: the loader tag is missing data-js / data-wasm');
}
const bindings = await import(js);
const instance = await bindings.default({ module_or_path: wasm });
window.wasmBindings = bindings;
dispatchEvent(new CustomEvent('TrunkApplicationStarted', { detail: { wasm: instance } }));

// Under `trunk serve`, Trunk injects an inline auto-reload script that the CSP
// blocks. Stand in for it: same endpoint, reload when Trunk says so.
const trunkReload = [...document.querySelectorAll('script:not([src])')].some((s) =>
  s.textContent.includes('.well-known/trunk/ws'),
);
if (trunkReload) {
  const url = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/.well-known/trunk/ws`;
  const connect = (reloadOnOpen) => {
    const ws = new WebSocket(url);
    if (reloadOnOpen) ws.onopen = () => location.reload();
    ws.onmessage = (event) => {
      try {
        if (JSON.parse(event.data).type === 'reload') location.reload();
      } catch (_) {
        /* not a Trunk message */
      }
    };
    // Trunk restarted: reconnect, then reload into the fresh build.
    ws.onclose = () => setTimeout(() => connect(true), 5000);
  };
  connect(false);
}
