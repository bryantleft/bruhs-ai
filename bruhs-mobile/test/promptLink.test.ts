import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parseIncomingPromptLink, parsePromptFragment, promptAppUrl, promptFragment } from '../src/promptLink.ts';

test('hosted and app links round-trip the same reviewed text', () => {
  const parsed = parseIncomingPromptLink('https://bruhs.ai/prompt#thread/id-1?prompt=Say%E2%80%8B+hello%0D%0A%F0%9F%91%A9%E2%80%8D%F0%9F%92%BB+%26+reset%3F');
  assert.deepEqual(parsed, { kind: 'prompt', link: { threadId: 'id-1', text: 'Say hello\n👩‍💻 & reset?', removedHiddenCharacters: true } });
  if (parsed.kind !== 'prompt') throw new Error('Expected a linked prompt');
  const app = parseIncomingPromptLink(promptAppUrl(parsed.link));
  assert.deepEqual(app, { kind: 'prompt', link: { ...parsed.link, removedHiddenCharacters: false } });
  assert.deepEqual(parsePromptFragment(promptFragment(parsed.link)), app);
});

test('fresh links carry text without execution settings or pairing authority', () => {
  assert.deepEqual(parseIncomingPromptLink('bruhs://new?prompt=Review+the+notes&permissions=full'), {
    kind: 'prompt', link: { threadId: null, text: 'Review the notes', removedHiddenCharacters: false },
  });
  for (const ignored of ['bruhs://pair?url=x', 'https://other.test/prompt#/new?prompt=x', 'bruhs://thread/id-1']) {
    assert.deepEqual(parseIncomingPromptLink(ignored), { kind: 'ignored' });
  }
});

test('duplicate, oversized, invisible, and malformed targets fail closed', () => {
  for (const invalid of [
    '#/new?prompt=first&prompt=second',
    '#/new?prompt=%E2%80%8B',
    '#/new?prompt=' + 'x'.repeat(8001),
    '#thread/id/path?prompt=test',
    '#thread/id#other?prompt=test',
  ]) assert.equal(parsePromptFragment(invalid).kind, 'invalid', invalid.slice(0, 80));
});
test('repository and model preferences survive the hosted to native handoff', () => {
  const link = parsePromptFragment('#/new?prompt=Review&repo=https%3A%2F%2Fgithub.com%2FExample%2FDemo.git&provider=CLAUDE&model=claude-sonnet-4-6');
  assert.equal(link.kind, 'prompt');
  if (link.kind !== 'prompt') return;
  assert.deepEqual(link.link, {
    threadId: null, text: 'Review', removedHiddenCharacters: false,
    repository: 'Example/Demo', provider: 'claude', model: 'claude-sonnet-4-6',
  });
  assert.deepEqual(parseIncomingPromptLink(promptAppUrl(link.link)), link);
});

test('pull request targets can stand alone and take precedence', () => {
  const link = parsePromptFragment('#/new?pr=Example%2FDemo%2342&repo=other%2Fdemo&provider=codex');
  assert.deepEqual(link, {
    kind: 'prompt', link: { threadId: null, text: '', removedHiddenCharacters: false, pullRequest: { repository: 'Example/Demo', number: 42 } },
  });
  if (link.kind !== 'prompt') return;
  assert.deepEqual(parseIncomingPromptLink(promptAppUrl(link.link)), link);
  for (const value of ['Example/Demo/0', 'Example/Demo/9007199254740992', 'https://github.com.example/Example/Demo/pull/42']) {
    assert.equal(parsePromptFragment(`#/new?pr=${encodeURIComponent(value)}`).kind, 'invalid');
  }
});
