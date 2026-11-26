export const HOSTED_PROMPT_URL = 'https://bruhs.ai/prompt';

export type LinkedPrompt = Readonly<{
  threadId: string | null;
  repository?: string;
  pullRequest?: { repository: string; number: number };
  provider?: string;
  model?: string;
  text: string;
  removedHiddenCharacters: boolean;
}>;

type PromptLinkResult =
  | { kind: 'ignored' }
  | { kind: 'invalid'; message: string }
  | { kind: 'prompt'; link: LinkedPrompt };

const hidden = /[\u0000-\u001f\u007f-\u009f\u00ad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180b-\u180f\u200b-\u200f\u202a-\u202e\u2060-\u206f\u3164\ufe00-\ufe0d\ufeff\uffa0\ufff0-\ufffb\u{1bca0}-\u{1bca3}\u{1d173}-\u{1d17a}\u{e0000}-\u{e0fff}]/u;
const emojiPart = /[\u2190-\u2bff\ufe0f\u{1f000}-\u{1faff}]/u;
const tooLong = { kind: 'invalid', message: 'Prompt links support up to 8,000 characters.' } as const;
const invalid = (message: string) => ({ kind: 'invalid', message } as const);

function repositorySlug(owner: string | undefined, rawName: string | undefined): string | undefined {
  const name = rawName?.replace(/\.git$/i, '');
  if (!owner || !/^[a-z0-9][a-z0-9-]{0,38}$/i.test(owner)) return;
  if (!name || !/^[a-z0-9._-]{1,100}$/i.test(name) || /^\.+$/.test(name)) return;
  return `${owner}/${name}`;
}

function parseRepository(value: string): string | undefined {
  const match = /^https?:\/\/(?:www\.)?github\.com\/([^/]+)\/([^/]+)\/*$/i.exec(value)
    ?? /^([^/]+)\/([^/]+)$/.exec(value);
  return repositorySlug(match?.[1], match?.[2]);
}

function parsePullRequest(value: string): LinkedPrompt['pullRequest'] {
  const match = /^https?:\/\/(?:www\.)?github\.com\/([^/]+)\/([^/]+)\/pull\/(\d+)(?:[/?#].*)?$/i.exec(value)
    ?? /^([^/#]+)\/([^/#]+)[/#](\d+)$/.exec(value);
  const repository = repositorySlug(match?.[1], match?.[2]);
  const number = Number(match?.[3]);
  if (repository && Number.isSafeInteger(number) && number > 0) return { repository, number };
}

// Keep the review text consistent with the desktop and browser link parser.
export function parsePromptFragment(fragment: string): PromptLinkResult {
  const boundary = fragment.indexOf('?');
  const route = (boundary === -1 ? fragment : fragment.slice(0, boundary)).replace(/\/+$/, '');
  const threadId = route.startsWith('#thread/') ? route.slice('#thread/'.length) : null;
  if (route !== '#/new' && !threadId) return { kind: 'ignored' };
  const query = boundary === -1 ? '' : fragment.slice(boundary + 1);
  if (query.length > 128 * 1024 || new TextEncoder().encode(query).byteLength > 128 * 1024) {
    return tooLong;
  }
  const params = new URLSearchParams(query);
  const prompts = params.getAll('prompt');
  if (!prompts.length && threadId) return { kind: 'ignored' };
  if (threadId && !/^[A-Za-z0-9_.:-]{1,128}$/.test(threadId)) {
    return { kind: 'invalid', message: 'The link has an invalid thread identifier.' };
  }
  if (prompts.length > 1) return { kind: 'invalid', message: 'The link contains more than one prompt.' };
  let target: Pick<LinkedPrompt, 'repository' | 'pullRequest' | 'provider' | 'model'> = {};
  if (!threadId) {
    const pr = params.get('pr')?.trim();
    const fields = pr ? ['pr'] : ['pr', 'repo', 'provider', 'model'];
    if (fields.some((key) => params.getAll(key).length > 1)) {
      return invalid('The link contains more than one value for a destination field.');
    }
    if (pr) {
      const pullRequest = parsePullRequest(pr);
      if (!pullRequest) return invalid('The link has an unreadable pull request. Use a GitHub pull request URL or owner/repository#number.');
      target = { pullRequest };
    } else {
      const repo = params.get('repo')?.trim();
      const repository = repo ? parseRepository(repo) : undefined;
      if (repo && !repository) return invalid('The link has an unreadable repository. Use a GitHub URL or owner/repository.');
      const provider = params.get('provider')?.trim();
      const model = params.get('model')?.trim();
      if ((provider && !/^[a-z0-9._-]{1,64}$/i.test(provider)) || (model && (
        new TextEncoder().encode(model).byteLength > 256 || /\s/.test(model) || hidden.test(model)
      ))) return invalid('The link has an unreadable model preference.');
      target = {
        ...(repository ? { repository } : {}),
        ...(provider ? { provider: provider.toLowerCase() } : {}),
        ...(model ? { model } : {}),
      };
    }
  }
  const raw = prompts[0] ?? '';
  if ([...raw].length > 8_000) return tooLong;
  const normalized = raw.replace(/\r\n?|[\u2028\u2029]/g, '\n');
  const visible = [...normalized].filter((char) => /[\n\t\u200d]/.test(char) || !hidden.test(char));
  const joined = visible.filter((char, index) =>
    char !== '\u200d' ||
    (emojiPart.test(visible[index - 1] ?? '') && emojiPart.test(visible[index + 1] ?? '')),
  ).join('');
  const removedHiddenCharacters = joined !== normalized;
  const text = joined.trim();
  if (!text && (removedHiddenCharacters || !target.pullRequest)) {
    return {
      kind: 'invalid',
      message: removedHiddenCharacters ? 'The link’s prompt contains only hidden characters.' : 'The link has no prompt.',
    };
  }
  return { kind: 'prompt', link: { threadId, text, removedHiddenCharacters, ...target } };
}

export function parseIncomingPromptLink(value: string): PromptLinkResult {
  const input = value.trim();
  if (input.startsWith(`${HOSTED_PROMPT_URL}#`)) return parsePromptFragment(input.slice(HOSTED_PROMPT_URL.length));
  const native = input.split('#', 1)[0] ?? '';
  if (/^bruhs:\/\/new(?:[/?]|$)/.test(native)) return parsePromptFragment(`#/${native.slice(8)}`);
  if (native.startsWith('bruhs://thread/')) return parsePromptFragment(`#${native.slice(8)}`);
  return { kind: 'ignored' };
}

export function promptFragment(link: Omit<LinkedPrompt, 'removedHiddenCharacters'>): string {
  const route = link.threadId === null ? '#/new' : `#thread/${link.threadId}`;
  const params = new URLSearchParams();
  if (link.text) params.set('prompt', link.text);
  if (link.pullRequest) params.set('pr', `${link.pullRequest.repository}#${link.pullRequest.number}`);
  if (link.repository) params.set('repo', link.repository);
  if (link.provider) params.set('provider', link.provider);
  if (link.model) params.set('model', link.model);
  return `${route}?${params}`;
}

export function promptAppUrl(link: LinkedPrompt): string {
  return `bruhs://${promptFragment(link).replace(/^#\/?/, '')}`;
}

export function pullRequestUrl(pullRequest: NonNullable<LinkedPrompt['pullRequest']>): string {
  return `https://github.com/${pullRequest.repository}/pull/${pullRequest.number}`;
}
