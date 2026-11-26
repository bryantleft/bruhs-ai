import { parsePromptFragment, promptAppUrl, promptFragment } from '../../../bruhs-mobile/src/promptLink.ts';

const choiceKey = 'bruhs-prompt-opener';

function workspaceUrl(value: string): URL | null {
  try {
    const url = new URL(value.trim());
    const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname) || url.hostname.endsWith('.localhost');
    if (url.protocol !== 'https:' && !(url.protocol === 'http:' && local)) return null;
    if (url.username || url.password || url.search || url.hash || url.hostname === 'bruhs.ai') return null;
    if (url.origin === location.origin && url.pathname.replace(/\/$/, '') === '/prompt') return null;
    return url;
  } catch {
    return null;
  }
}

function readChoice(): string | null {
  try { return localStorage.getItem(choiceKey); } catch { return null; }
}

function rememberChoice(choice: string): void {
  try { localStorage.setItem(choiceKey, choice); } catch { /* The link also works without storage. */ }
}

const title = document.querySelector<HTMLElement>('[data-prompt-title]')!;
const status = document.querySelector<HTMLElement>('[data-prompt-status]')!;
const warning = document.querySelector<HTMLElement>('[data-prompt-warning]')!;
const preview = document.querySelector<HTMLElement>('[data-prompt-text]')!;
const destination = document.querySelector<HTMLElement>('[data-prompt-destination]')!;
const actions = document.querySelector<HTMLElement>('[data-prompt-actions]')!;
const app = document.querySelector<HTMLAnchorElement>('[data-open-app]')!;
const browser = document.querySelector<HTMLButtonElement>('[data-open-browser]')!;
const form = document.querySelector<HTMLFormElement>('[data-prompt-form]')!;
const input = document.querySelector<HTMLInputElement>('#prompt-workspace')!;
const error = document.querySelector<HTMLElement>('#prompt-workspace-error')!;
const remember = document.querySelector<HTMLInputElement>('[data-remember]')!;
const footer = document.querySelector<HTMLElement>('[data-prompt-footer]')!;

function render(followChoice = false): void {
  const result = parsePromptFragment(location.hash);
  const ready = result.kind === 'prompt';
  actions.hidden = footer.hidden = preview.hidden = destination.hidden = !ready;
  form.hidden = warning.hidden = error.hidden = true;
  input.removeAttribute('aria-invalid');
  browser.setAttribute('aria-expanded', 'false');
  app.removeAttribute('href');
  preview.textContent = destination.textContent = '';
  title.textContent = result.kind === 'invalid' ? 'Could not open the prompt' : 'Open a prompt';
  if (result.kind !== 'prompt') {
    status.textContent = result.kind === 'invalid' ? result.message : 'Open the full link to see its draft.';
    return;
  }

  const link = result.link;
  if (link.pullRequest) title.textContent = 'Open a pull request';
  const preferred = readChoice();
  const target = workspaceUrl(preferred ?? '');
  status.textContent = link.text ? 'It opens as a draft. Review it before sending.' : 'Review this pull request in Bruhs.';
  preview.textContent = link.text;
  preview.hidden = !link.text;
  warning.hidden = !link.removedHiddenCharacters;
  const placement = link.pullRequest ? `${link.pullRequest.repository} #${link.pullRequest.number}`
    : link.repository ?? (link.threadId ? 'Existing thread' : 'New thread');
  const requestedModel = [link.provider, link.model].filter(Boolean).join(' / ');
  destination.textContent = requestedModel ? `${placement} · ${requestedModel}` : placement;
  const native = promptAppUrl(link);
  app.href = native;
  app.onclick = () => { if (remember.checked) rememberChoice('app'); };
  input.value = target?.href ?? '';
  const continueTo = (url: URL) => {
    if (remember.checked) rememberChoice(url.href);
    url.hash = promptFragment(link);
    location.assign(url.href);
  };
  browser.onclick = () => {
    form.hidden = !form.hidden;
    browser.setAttribute('aria-expanded', String(!form.hidden));
    if (!form.hidden) input.focus();
  };
  form.onsubmit = (event) => {
    event.preventDefault();
    const url = workspaceUrl(input.value);
    if (!url) {
      error.textContent = 'Use a complete HTTPS workspace URL or a local Bruhs address.';
      error.hidden = false;
      input.setAttribute('aria-invalid', 'true');
      input.focus();
      return;
    }
    error.hidden = true;
    input.removeAttribute('aria-invalid');
    continueTo(url);
  };
  if (followChoice && preferred === 'app') location.assign(native);
  if (followChoice && target) continueTo(target);
}

render(true);
addEventListener('hashchange', () => render());
