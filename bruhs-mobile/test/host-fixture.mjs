// Synthetic loopback acceptance host. It never opens a provider, repository,
// shell, or real Bruhs process. No production credentials are accepted.
import { createServer } from 'node:http';
import { randomUUID } from 'node:crypto';

const port = Number(process.env.BRUHS_FIXTURE_PORT ?? 18441);
const threadId = '96c949f3-499b-41df-980b-f17b7004505a';
const taskId = '33f6b240-76f1-47ab-aa8b-b0dbd140d874';
const secret = 'synthetic_bruhs_pairing_offer_for_testing_1234';
const token = 'synthetic-desktop-token';
const taskToken = 'synthetic-task-owner-token';
const guestToken = 'synthetic-task-guest-token';
const now = () => Date.now();
let state;
function reset() {
  state = {
    scopes: ['read', 'prompt', 'approve', 'diff'],
    revoked: false,
    expired: false,
    offline: false,
    next: 0,
    receipt: null,
    pendingBody: null,
    mode: 'normal',
    permission: true,
    generation: 4,
    reviewHash: 'fixture-review-hash-4',
    nativeRequests: [],
    nativeDispatches: 0,
    taskRequests: [],
    taskState: 'completed',
    messages: [
      { role: 'user', text: 'Review the mobile companion before shipping.', at: now() },
      {
        role: 'assistant',
        text: 'Native pairing, secure drafts and exact approval receipts are ready to review.',
        at: now(),
      },
    ],
  };
}
reset();
const device = () => ({
  id: threadId,
  name: 'Acceptance phone',
  scopes: state.scopes,
  expires_at_ms: state.expired ? 1 : now() + 3600_000,
  last_seen_ms: now(),
  revoked: state.revoked,
});
const thread = () => ({
  id: threadId,
  title: 'Mobile companion review',
  project: '/synthetic/bruhs-ai',
  updated_at_ms: now(),
  state: state.permission ? 'needs_approval' : 'idle',
});
const taskSnapshot = (owner) => ({
  can_control: owner,
  revision: 1,
  specs: [],
  guest_proposals: [],
  tasks: [
    {
      id: taskId,
      parent_id: null,
      title: 'Ship the native companion',
      project_path: owner ? '/synthetic/bruhs-ai' : '',
      working_path: owner ? '/synthetic/bruhs-ai' : '',
      model: 'synthetic/default',
      permission_mode: 'default',
      state: state.taskState,
      messages: state.messages,
      live_output: '',
      error: null,
      approvals:
        state.taskState === 'waiting_approval'
          ? [
              {
                id: 'task-approval-1',
                tool: 'Write',
                input: { file: 'src/mobile.ts', content: 'synthetic' },
              },
            ]
          : [],
      updated_at: now(),
      resume_at: null,
      auto_resume_on_limit: false,
    },
  ],
});

const server = createServer(async (req, res) => {
  const url = new URL(req.url ?? '/', `http://127.0.0.1:${port}`);
  const origin = req.headers.origin;
  if (origin && !['http://localhost:18440', 'http://127.0.0.1:18440'].includes(origin)) {
    res.writeHead(403);
    res.end();
    return;
  }
  if (origin) res.setHeader('Access-Control-Allow-Origin', origin);
  res.setHeader('Access-Control-Allow-Headers', 'Authorization,Content-Type,Cache-Control');
  res.setHeader('Access-Control-Allow-Methods', 'GET,POST,OPTIONS');
  res.setHeader('Cache-Control', 'no-store');
  const json = (value, status = 200) => {
    res.writeHead(status, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(value));
  };
  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }
  let body;
  if (req.method === 'POST') {
    let text = '';
    for await (const chunk of req) {
      text += chunk.toString();
      if (text.length > 65536) {
        json({ error: 'fixture frame too large' }, 413);
        return;
      }
    }
    try {
      body = JSON.parse(text || '{}');
    } catch {
      json({ error: 'invalid fixture JSON' }, 400);
      return;
    }
  }
  if (url.pathname === '/__fixture/state') {
    if (req.method === 'POST') {
      if (body.reset) reset();
      Object.assign(
        state,
        Object.fromEntries(
          Object.entries(body).filter(([key]) =>
            [
              'scopes',
              'revoked',
              'expired',
              'offline',
              'mode',
              'permission',
              'taskState',
              'generation',
              'reviewHash',
            ].includes(key),
          ),
        ),
      );
    }
    json(state);
    return;
  }
  if (state.offline) {
    req.socket.destroy();
    return;
  }
  if (url.pathname === '/api/mobile/pair') {
    if (body?.secret !== secret) {
      json({ error: 'fixture pairing offer is invalid' }, 401);
      return;
    }
    state.revoked = false;
    state.expired = false;
    json({ token, device: device(), next_sequence: state.next });
    return;
  }
  const native = url.pathname.startsWith('/api/mobile/');
  const owner = req.headers.authorization === `Bearer ${taskToken}`;
  const guest = req.headers.authorization === `Bearer ${guestToken}`;
  if (
    native &&
    (req.headers.authorization !== `Bearer ${token}` || state.revoked || state.expired)
  ) {
    json({ error: 'This pairing expired or was revoked.' }, 401);
    return;
  }
  if (!native && !owner && !guest) {
    json({ error: 'Invalid task token.' }, 403);
    return;
  }
  switch (url.pathname) {
    case '/api/mobile/session':
      json({ device: device(), next_sequence: state.next, last_command: state.receipt });
      return;
    case '/api/mobile/threads':
      json({ kind: 'threads', value: { threads: [thread()], next_offset: null } });
      return;
    case '/api/mobile/thread':
      json({
        kind: 'thread',
        value: {
          thread: thread(),
          messages: state.messages.map((message, index) => ({
            index,
            role: message.role,
            text: message.text,
            byte_offset: 0,
            full_bytes: Buffer.byteLength(message.text),
            streaming: false,
            reference: `fixture-message-${index}`,
          })),
          next_before: null,
          permission: state.permission
            ? {
                request_id: 'native-request-4',
                turn_generation: state.generation,
                tool: 'Write mobile companion',
                input: '{"file":"src/mobile.ts","content":"synthetic local fixture"}',
                review_hash: state.reviewHash,
                can_allow: true,
              }
            : null,
          queued_prompts: 0,
          laptop_action_required: false,
        },
      });
      return;
    case '/api/mobile/diff':
    case '/api/mobile/message':
      json({
        kind: 'text',
        value: {
          text: 'diff --git a/src/mobile.ts b/src/mobile.ts\n+export const nativeCompanion = true;\n',
          offset: 0,
          next_offset: null,
          revision: 'fixture-revision-1',
          source_truncated: false,
        },
      });
      return;
    case '/api/mobile/disconnect':
      state.revoked = true;
      json({ disconnected: true });
      return;
    case '/api/mobile/command': {
      state.nativeRequests.push(body);
      if (state.receipt?.id === body.id) {
        if (state.pendingBody !== JSON.stringify(body)) {
          json({ error: 'UUID payload changed' }, 409);
          return;
        }
        if (state.receipt.outcome.status !== 'pending') {
          json(state.receipt);
          return;
        }
      } else {
        if (body.sequence !== state.next) {
          json({ error: 'Sequence changed' }, 409);
          return;
        }
        state.next++;
        state.pendingBody = JSON.stringify(body);
        state.receipt = { id: body.id, sequence: body.sequence, outcome: { status: 'pending' } };
        if (state.mode === 'native-drop-once') {
          state.mode = 'normal';
          req.socket.destroy();
          return;
        }
        if (state.mode === 'native-pending-once') {
          state.mode = 'normal';
          json({ error: 'Synthetic receipt interrupted after reservation' }, 500);
          return;
        }
      }
      if (
        body.action.kind === 'permission' &&
        (body.action.turn_generation !== state.generation ||
          body.action.review_hash !== state.reviewHash ||
          body.action.request_id !== 'native-request-4')
      )
        state.receipt.outcome = {
          status: 'rejected',
          reason: 'This approval changed. Review its current request.',
        };
      else {
        state.nativeDispatches++;
        if (body.action.kind === 'permission') state.permission = false;
        else state.messages.push({ role: 'user', text: body.action.text, at: now() });
        state.receipt.outcome = { status: 'accepted', queued: false };
      }
      json(state.receipt);
      return;
    }
    case '/api/tasks':
      json(taskSnapshot(owner));
      return;
    case '/api/commands': {
      if (!owner) {
        json({ error: 'Owner access required.' }, 403);
        return;
      }
      state.taskRequests.push(body);
      if (body.action === 'send')
        state.messages.push({ role: 'user', text: body.prompt, at: now() });
      if (body.action === 'cancel') state.taskState = 'cancelled';
      if (body.action === 'approval') state.taskState = 'running';
      if (state.mode === 'task-malformed-once') {
        state.mode = 'normal';
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end('{');
        return;
      }
      json(taskSnapshot(owner));
      return;
    }
    case '/api/proposals':
      state.taskRequests.push(body);
      json({ id: randomUUID(), task_id: taskId, text: body.text, at: now() });
      return;
    default:
      json({ error: 'Unknown synthetic route' }, 404);
  }
});
server.listen(port, '127.0.0.1', () =>
  console.log(
    `Synthetic mobile host listening on 127.0.0.1:${port}. Pairing fragment: #pair=${secret}. Task owner: ${taskToken}. Guest: ${guestToken}.`,
  ),
);
