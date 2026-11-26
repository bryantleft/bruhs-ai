import assert from 'node:assert/strict';
import { test } from 'node:test';
import { z } from 'zod';
import { HostClient, pairDesktop, type Transport } from '../src/client.ts';
import {
  acknowledgeDraft,
  commandSchema,
  draftKey,
  nextCommand,
  originOf,
  outcomeSchema,
  parsePairing,
  requestUrl,
  sessionSchema,
  threadSnapshotSchema,
  validateHost,
  type NativeAction,
} from '../src/domain.ts';
import { canMutate, checkSelection, recoverDelivery } from '../src/delivery.ts';
import { accepted, command, desktop, otherUuid, session, tasks, uuid } from './fixtures.ts';

test('production origins require a root HTTPS address and forbid embedded credentials', () => {
  assert.deepEqual(originOf('https://LAPTOP.example:9418/'), { ok: true, value: desktop.origin });
  for (const value of [
    'http://laptop.example',
    'https://name:pass@laptop.example',
    'https://laptop.example/path',
    'https://laptop.example?token=leak',
    'https://laptop.example/#pair=leak',
    'file:///etc/hosts',
    'javascript:alert(1)',
    'https://laptop.example\\evil',
  ])
    assert.equal(originOf(value).ok, false, value);
});

test('development permits literal loopback HTTP only', () => {
  for (const host of ['localhost', '127.0.0.1', '[::1]'])
    assert.equal(originOf(`http://${host}:18441`, true).ok, true);
  for (const host of ['192.168.1.50', 'localhost.example', '127.0.0.1.example'])
    assert.equal(originOf(`http://${host}`, true).ok, false);
  assert.equal(originOf('http://127.0.0.1:18441').ok, false);
});

test('one-use QR/manual/deep links retain the secret in the fragment only', () => {
  const secret = 'S'.repeat(43),
    link = `${desktop.origin}/#pair=${secret}`;
  assert.deepEqual(parsePairing(link), { ok: true, value: { origin: desktop.origin, secret } });
  assert.deepEqual(
    parsePairing(`bruhs://pair?url=${encodeURIComponent(link)}`),
    parsePairing(link),
  );
  for (const bad of [
    link + '&pair=' + secret,
    link + '&other=1',
    `${desktop.origin}/?pair=${secret}`,
    `${desktop.origin}/#pair=short`,
    `${desktop.origin}/other#pair=${secret}`,
    `bruhs://other?url=${encodeURIComponent(link)}`,
    `bruhs://pair?url=${encodeURIComponent(link)}&url=${encodeURIComponent(link)}`,
    `bruhs://pair/other?url=${encodeURIComponent(link)}`,
  ])
    assert.equal(parsePairing(bad).ok, false, bad);
});

test('saved host validation rejects noncanonical origin or malformed bearer', () => {
  assert.equal(validateHost(desktop).ok, true);
  for (const change of [
    { origin: desktop.origin + '/' },
    { origin: 'http://laptop.example' },
    { token: 'unsafe\r\nAuthorization: value' },
    { token: '' },
    { authority: 'cloud' },
  ])
    assert.equal(validateHost({ ...desktop, ...change }).ok, false);
});

test('credentials remain inside the exact origin and authority route family', () => {
  assert.deepEqual(requestUrl(desktop, '/api/mobile/session'), {
    ok: true,
    value: `${desktop.origin}/api/mobile/session`,
  });
  for (const path of [
    'https://attacker.example/api/mobile/session',
    '//attacker.example/api/mobile/session',
    '/api/tasks',
    '/api/mobile/../../api/commands',
    '/api/mobile/session#leak',
  ])
    assert.equal(requestUrl(desktop, path).ok, false, path);
  for (const path of [
    '/api/mobile/session',
    '/api/mobile/command',
    '/api/commands/extra',
    '/api/commands/../mobile/session',
  ])
    assert.equal(requestUrl(tasks, path).ok, false, path);
});

test('cross-authority calls never reach transport or expose a token', async () => {
  let calls = 0;
  const transport: Transport = async () => {
    calls++;
    throw new Error('should not run');
  };
  assert.equal((await new HostClient(desktop, transport).tasks()).ok, false);
  assert.equal((await new HostClient(tasks, transport).session()).ok, false);
  assert.equal(
    (
      await new HostClient(desktop, transport).request(
        'https://attacker.example/api/mobile/session',
        z.unknown(),
      )
    ).ok,
    false,
  );
  assert.equal(calls, 0);
});

test('pairing posts the one-use secret once without bearer or query leakage', async () => {
  const seen: { url: string; init: Parameters<Transport>[1] }[] = [];
  const transport: Transport = async (url, init) => {
    seen.push({ url, init });
    return {
      status: 200,
      text: JSON.stringify({ token: 'paired-token', device: session.device, next_sequence: 0 }),
    };
  };
  const result = await pairDesktop(
    transport,
    { origin: desktop.origin, secret: 'S'.repeat(43) },
    'Phone',
  );
  assert.equal(result.ok, true);
  assert.equal(seen.length, 1);
  assert.equal(seen[0]?.url, desktop.origin + '/api/mobile/pair');
  assert.equal(seen[0]?.init.token, undefined);
  const body = seen[0]?.init.body;
  assert(typeof body === 'string');
  assert.deepEqual(JSON.parse(body), { name: 'Phone', secret: 'S'.repeat(43) });
  assert.equal(
    (
      await pairDesktop(
        transport,
        { origin: 'https://user:pass@attacker.example', secret: 'S'.repeat(43) },
        'Phone',
      )
    ).ok,
    false,
  );
  assert.equal(seen.length, 1);
});

test('command retries preserve the exact UUID, sequence, body and credential scope', async () => {
  const seen: { url: string; init: Parameters<Transport>[1] }[] = [];
  const transport: Transport = async (url, init) => {
    seen.push({ url, init });
    return { status: 200, text: JSON.stringify(accepted) };
  };
  const client = new HostClient(desktop, transport);
  await client.command(command);
  await client.command(command);
  assert.equal(seen.length, 2);
  assert.equal(seen[0]?.init.body, seen[1]?.init.body);
  assert.equal(seen[0]?.init.body, JSON.stringify(command));
  assert.equal(seen[0]?.init.token, desktop.token);
  assert.equal(seen[0]?.url, desktop.origin + '/api/mobile/command');
});

test('approval payload requires the exact request, turn generation and review hash', () => {
  const action: NativeAction = {
    kind: 'permission',
    thread_id: uuid,
    turn_generation: 8,
    request_id: 'permission-2',
    review_hash: 'sha256-review',
    decision: 'allow',
  };
  assert.deepEqual(nextCommand(otherUuid, 7, action), {
    ok: true,
    value: { id: otherUuid, sequence: 7, action },
  });
  for (const missing of ['request_id', 'turn_generation', 'review_hash']) {
    const damaged: Record<string, unknown> = { ...action };
    delete damaged[missing];
    assert.equal(
      commandSchema.safeParse({ id: otherUuid, sequence: 7, action: damaged }).success,
      false,
    );
  }
  assert.equal(nextCommand(otherUuid, Number.MAX_SAFE_INTEGER + 1, command.action).ok, false);
});

test('a phone answer round-trips with optional attachment identifiers', async () => {
  const action: NativeAction = {
    kind: 'answer_question',
    thread_id: uuid,
    turn_generation: 8,
    request_id: 'question-1',
    review_hash: '0'.repeat(64),
    reply: { type: 'answer', answers: [{ selected: [0], text: 'Add the examples' }] },
  };
  const command = { id: otherUuid, sequence: 7, action };
  assert.deepEqual(commandSchema.parse(command), command);
  const withFile = { ...command, action: {
    ...action, reply: { type: 'answer', answers: [{ selected: [], text: '', attachments: [uuid] }] },
  } };
  assert.deepEqual(commandSchema.parse(withFile), withFile);
  assert.equal(commandSchema.safeParse({ ...command, action: {
    ...action, reply: { type: 'answer', answers: [{ selected: [0], text: '', attachments: ['/tmp/photo.png'] }] },
  } }).success, false);

  const snapshot = {
    kind: 'thread',
    value: {
      thread: { id: uuid, title: 'Question fixture', project: 'Test', updated_at_ms: 1, state: 'laptop_action' },
      messages: [], next_before: null, permission: null, queued_prompts: 0, laptop_action_required: true,
    },
  };
  assert.equal(threadSnapshotSchema.parse(snapshot).value.question, undefined);
  const question = {
    request_id: action.request_id, turn_generation: action.turn_generation, review_hash: action.review_hash,
    questions: [{ question: 'What should I include?', header: 'Contents', options: [], multiSelect: false, allowCustom: true }],
    responding: false, error: null, can_attach: true,
  };
  assert.deepEqual(threadSnapshotSchema.parse({ ...snapshot, value: { ...snapshot.value, question } }).value.question, question);
  const file = new Blob(['fixture bytes']);
  const calls: Array<{ url: URL; init: Parameters<Transport>[1] }> = [];
  const client = new HostClient(desktop, async (url, init) => {
    calls.push({ url: new URL(url), init });
    return { status: 200, text: '{"ok":true}' };
  });
  assert((await client.questionFile(uuid, question, 0, otherUuid, { name: 'Notes & examples.txt', body: file })).ok);
  assert((await client.questionFile(uuid, question, 0, otherUuid)).ok);
  assert.equal(calls[0]?.url.pathname, '/api/mobile/question-file');
  assert.equal(calls[0]?.url.searchParams.get('name'), 'Notes & examples.txt');
  assert.equal(calls[0]?.url.searchParams.get('review_hash'), question.review_hash);
  assert.equal(calls[0]?.url.searchParams.get('upload_id'), otherUuid);
  assert.equal(calls[0]?.init.token, desktop.token);
  assert.equal(calls[0]?.init.method, 'POST');
  assert.equal(calls[0]?.init.contentType, 'application/octet-stream');
  assert.equal(calls[0]?.init.body, file);
  assert.equal(calls[1]?.init.method, 'DELETE');
  assert.equal(calls[1]?.init.body, undefined);
});

test('malformed, 5xx, and interrupted successful task responses retain uncertain delivery', async () => {
  for (const response of [
    { status: 200, text: '{}' },
    { status: 200, text: '<html>proxy error</html>' },
    { status: 500, text: '{"error":"write failed after acceptance"}' },
    { status: 408, text: '{}' },
  ]) {
    const client = new HostClient(tasks, async () => response);
    const result = await client.taskCommand({ action: 'send', id: uuid, prompt: 'Continue' });
    assert.equal(result.ok, false);
    if (!result.ok) assert.equal(result.error.delivery, 'uncertain');
  }
  const result = await new HostClient(tasks, async () => {
    throw new Error('connection lost');
  }).taskCommand({ action: 'cancel', id: uuid });
  if (!result.ok) assert.equal(result.error.delivery, 'uncertain');
  else assert.fail('network failure accepted');
});

test('definitive task 4xx rejection permits a new action after state review', async () => {
  for (const status of [400, 401, 403, 409, 429]) {
    const result = await new HostClient(tasks, async () => ({
      status,
      text: '{"error":"rejected"}',
    })).taskCommand({ action: 'cancel', id: uuid });
    assert.equal(result.ok, false);
    if (!result.ok) assert.equal(result.error.delivery, 'rejected');
  }
});

test('external response validation rejects malformed session state', async () => {
  const result = await new HostClient(desktop, async () => ({
    status: 200,
    text: JSON.stringify({ ...session, next_sequence: -1 }),
  })).session();
  assert.equal(result.ok, false);
  assert.equal(
    sessionSchema.safeParse({ ...session, device: { ...session.device, scopes: ['finance'] } })
      .success,
    false,
  );
});

test('a thread read rejects another conversation even when its snapshot is valid', async () => {
  const body = {
    kind: 'thread',
    value: {
      thread: { id: uuid, title: 'Work', project: 'Project', updated_at_ms: 0, state: 'idle' },
      messages: [],
      next_before: null,
      permission: null,
      queued_prompts: 0,
      laptop_action_required: false,
    },
  };
  const host = new HostClient(desktop, async () => ({ status: 200, text: JSON.stringify(body) }));
  assert.equal((await host.thread(uuid)).ok, true);
  const different = await host.thread(otherUuid);
  assert.equal(different.ok, false);
  if (!different.ok) assert.equal(different.error.kind, 'conflict');
});

test('a proxy HTML authentication rejection still ends the rejected authority session', async () => {
  const desktopResult = await new HostClient(desktop, async () => ({
    status: 401,
    text: '<h1>Expired</h1>',
  })).session();
  assert.equal(desktopResult.ok, false);
  if (!desktopResult.ok) assert.equal(desktopResult.error.kind, 'unauthorized');
  const guestResult = await new HostClient(tasks, async () => ({
    status: 403,
    text: 'Forbidden',
  })).tasks();
  assert.equal(guestResult.ok, false);
  if (!guestResult.ok) assert.equal(guestResult.error.kind, 'forbidden');
});

test('read cancellation reaches transport and leaves the pending command intact', async () => {
  const outer = new AbortController();
  let observed = false;
  const transport: Transport = async (_url, init) =>
    new Promise((_resolve, reject) => {
      init.signal.addEventListener('abort', () => {
        observed = true;
        reject(new Error('aborted'));
      });
    });
  const read = new HostClient(desktop, transport, outer.signal).session();
  outer.abort();
  const result = await read;
  assert.equal(observed, true);
  assert.equal(result.ok, false);
  assert.deepEqual(recoverDelivery(command, session), { kind: 'retry', command });
});

test('receipt recovery never synthesizes a new ID or replays an advanced sequence', () => {
  assert.deepEqual(recoverDelivery(command, session), { kind: 'retry', command });
  for (const status of ['pending', 'accepted', 'rejected', 'unconfirmed']) {
    const outcome =
      status === 'accepted'
        ? { status, queued: true }
        : status === 'rejected'
          ? { status, reason: 'changed request' }
          : { status };
    const receipt = { ...accepted, outcome: outcomeSchema.parse(outcome) };
    assert.deepEqual(
      recoverDelivery(command, { ...session, next_sequence: 8, last_command: receipt }),
      { kind: 'receipt', receipt },
    );
  }
  assert.equal(recoverDelivery(command, { ...session, next_sequence: 8 }).kind, 'review');
  assert.equal(
    recoverDelivery(command, {
      ...session,
      last_command: { ...accepted, id: uuid },
      next_sequence: 8,
    }).kind,
    'review',
  );
});

test('draft acknowledgement preserves edits and host/item isolation', () => {
  assert.equal(
    acknowledgeDraft('  Continue the implementation. ', command.action.text, accepted),
    '',
  );
  assert.equal(
    acknowledgeDraft('Continue and keep this new detail', command.action.text, accepted),
    'Continue and keep this new detail',
  );
  assert.equal(
    acknowledgeDraft(command.action.text, command.action.text, {
      ...accepted,
      outcome: { status: 'unconfirmed' },
    }),
    command.action.text,
  );
  assert.notEqual(draftKey(desktop, uuid), draftKey(tasks, uuid));
  assert.notEqual(draftKey(desktop, uuid), draftKey(desktop, otherUuid));
  assert.equal(checkSelection(tasks, desktop).ok, false);
});

test('offline, expired, revoked, read-only, and pending sessions cannot mutate', () => {
  assert.equal(canMutate(desktop, true, null, session, 'approve'), true);
  assert.equal(canMutate(desktop, false, null, session, 'approve'), false);
  assert.equal(canMutate(desktop, true, command, session, 'prompt'), false);
  assert.equal(
    canMutate(
      desktop,
      true,
      null,
      { ...session, device: { ...session.device, revoked: true } },
      'prompt',
    ),
    false,
  );
  assert.equal(
    canMutate(
      desktop,
      true,
      null,
      { ...session, device: { ...session.device, expires_at_ms: 0 } },
      'prompt',
    ),
    false,
  );
  assert.equal(
    canMutate(
      desktop,
      true,
      null,
      { ...session, device: { ...session.device, scopes: ['read'] } },
      'prompt',
    ),
    false,
  );
  assert.equal(canMutate(tasks, true, null, session, 'prompt'), false);
});

test('linked draft routing requires its own grant and validates the exact response identifier', async () => {
  assert.equal(canMutate(desktop, true, null, session, 'linked_drafts'), false);
  const granted = { ...session, device: { ...session.device, scopes: [...session.device.scopes, 'linked_drafts' as const] } };
  assert.equal(canMutate(desktop, true, null, granted, 'linked_drafts'), true);
  const action = { kind: 'open_linked_draft', target: { repository: 'example/docs' } };
  assert.equal(commandSchema.safeParse({id: uuid, sequence: 1, action}).success, true);
  assert.equal(commandSchema.safeParse({id: uuid, sequence: 1, action: { ...action, target: { ...action.target, path: '/tmp', prompt: 'Hello' } }}).success, false);
  const client = new HostClient(desktop, async url => {
    assert.equal(new URL(url).searchParams.get('link_id'), uuid);
    return { status: 200, text: JSON.stringify({ kind: 'linked_draft', value: { link_id: otherUuid, status: 'preparing' } }) };
  });
  assert.equal((await client.linkedDraft(uuid)).ok, false);
});
