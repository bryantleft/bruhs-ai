export function widgetTheme() {
  const css = getComputedStyle(document.documentElement);
  const value = (name, fallback) => css.getPropertyValue(name).trim() || fallback;
  const scheme = css.colorScheme.includes('dark') ? 'dark' : 'light';
  const background = value('--color-longan-950', scheme === 'dark' ? '#141210' : '#fffaf5');
  const foreground = value('--color-rambutan-200', scheme === 'dark' ? '#dfcfc3' : '#352b24');
  const accent = value('--color-persimmon-400', '#f18a53');
  const font = value('--font-neon', 'ui-monospace, monospace');
  return {
    'color-scheme': scheme,
    '--bruhs-background': background, '--bruhs-text': foreground,
    '--bruhs-accent': accent, '--bruhs-font': font,
    '--background': background, '--foreground': foreground, '--primary': accent,
    '--font-sans': font, '--border': value('--color-longan-800', '#44372c'),
    '--muted': value('--color-rambutan-400', '#ad9789'),
  };
}

// The child is opaque. Only messages from this exact iframe and nonce cross
// into application code; the payload can never invoke arbitrary app methods.
export function connectWidget(frame, nonce, callback) {
  let state = { fields: {}, custom: null, scroll_x: 0, scroll_y: 0 };
  let stopped = false;

  const initialize = () => {
    if (!stopped && frame.contentWindow) frame.contentWindow.postMessage({ bruhs: 1, type: 'init', state, theme: widgetTheme() }, '*');
  };
  const receive = event => {
    const packet = event.data;
    if (stopped || event.source !== frame.contentWindow || !packet || packet.bruhs !== 1 || packet.nonce !== nonce) return;
    if (!['ready', 'resize', 'state', 'error', 'action'].includes(packet.type)) return;
    if (packet.type === 'state') state = packet.state;
    try { callback(JSON.stringify(packet)); }
    catch (error) { callback(JSON.stringify({ bruhs: 1, nonce, type: 'error', message: `Could not read widget data: ${error.message}` })); }
    if (packet.type === 'ready') initialize();
  };
  const observer = new MutationObserver(initialize);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] });
  const appearance = matchMedia('(prefers-color-scheme: dark)');
  appearance.addEventListener('change', initialize);
  addEventListener('message', receive);
  frame.addEventListener('load', initialize);
  return {
    initialize(nextState) { state = nextState; initialize(); },
    dispose() {
      stopped = true;
      removeEventListener('message', receive);
      frame.removeEventListener('load', initialize);
      observer.disconnect();
      appearance.removeEventListener('change', initialize);
    },
  };
}

export function initializeWidget(handle, state) { handle.initialize(state); }
export function disposeWidget(handle) { handle.dispose(); }

export function watchFrameInteraction(frame, callback) {
  let stopped=false;
  const focus = () => queueMicrotask(() => { if (!stopped && document.activeElement === frame) callback(); });
  addEventListener('blur', focus);
  return () => {stopped=true;removeEventListener('blur', focus);};
}
export function disposeWatcher(dispose) { dispose(); }

export function fullscreen(element) {
  if (document.fullscreenElement === element) return document.exitFullscreen();
  if (!element.requestFullscreen) return Promise.reject(Error('Fullscreen is unavailable in this browser.'));
  return element.requestFullscreen();
}

export function openDialog(element) { if (!element.open) element.showModal(); }
