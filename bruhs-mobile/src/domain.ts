import { z } from 'zod';

export type Failure = Readonly<{
  kind: 'invalid' | 'network' | 'unauthorized' | 'forbidden' | 'storage' | 'conflict';
  message: string;
  delivery?: 'uncertain' | 'rejected';
}>;
export type Result<T> = Readonly<{ ok: true; value: T } | { ok: false; error: Failure }>;
export const success = <T>(value: T): Result<T> => ({ ok: true, value });
export const failure = (
  kind: Failure['kind'],
  message: string,
  delivery?: Failure['delivery'],
): Result<never> => ({ ok: false, error: { kind, message, ...(delivery ? { delivery } : {}) } });
export const messageOf = (error: unknown): string =>
  error instanceof Error ? error.message : 'The operation failed. Check the host and retry.';

const integer = z.number().int().nonnegative().safe();
const scope = z.enum(['read', 'prompt', 'approve', 'diff']);
export const deviceSchema = z.object({
  id: z.string().uuid(),
  name: z.string(),
  scopes: z.array(scope),
  expires_at_ms: integer,
  last_seen_ms: integer.nullable(),
  revoked: z.boolean(),
});
export const outcomeSchema = z.discriminatedUnion('status', [
  z.object({ status: z.literal('pending') }),
  z.object({ status: z.literal('accepted'), queued: z.boolean() }),
  z.object({ status: z.literal('rejected'), reason: z.string() }),
  z.object({ status: z.literal('unconfirmed') }),
]);
export const receiptSchema = z.object({
  id: z.string().uuid(),
  sequence: integer,
  outcome: outcomeSchema,
});
export const sessionSchema = z.object({
  device: deviceSchema,
  next_sequence: integer,
  last_command: receiptSchema.nullable(),
});
export const pairSchema = z.object({
  token: z.string().min(1),
  device: deviceSchema,
  next_sequence: integer,
});
export const nativeActionSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('prompt'), thread_id: z.string(), text: z.string() }),
  z.object({
    kind: z.literal('permission'),
    thread_id: z.string(),
    turn_generation: integer,
    request_id: z.string(),
    review_hash: z.string(),
    decision: z.enum(['allow', 'deny']),
  }),
]);
export const commandSchema = z.object({
  id: z.string().uuid(),
  sequence: integer,
  action: nativeActionSchema,
});
export const threadSchema = z.object({
  id: z.string(),
  title: z.string(),
  project: z.string(),
  updated_at_ms: integer,
  state: z.enum(['idle', 'running', 'needs_approval', 'laptop_action']),
});
export const threadsSchema = z.object({
  kind: z.literal('threads'),
  value: z.object({ threads: z.array(threadSchema), next_offset: integer.nullable() }),
});
export const permissionSchema = z.object({
  request_id: z.string(),
  turn_generation: integer,
  tool: z.string(),
  input: z.string(),
  review_hash: z.string(),
  can_allow: z.boolean(),
});
export const threadSnapshotSchema = z.object({
  kind: z.literal('thread'),
  value: z.object({
    thread: threadSchema,
    messages: z.array(
      z.object({
        index: integer,
        role: z.string(),
        text: z.string(),
        byte_offset: integer,
        full_bytes: integer,
        streaming: z.boolean(),
        reference: z.string(),
      }),
    ),
    next_before: integer.nullable(),
    permission: permissionSchema.nullable(),
    queued_prompts: integer,
    laptop_action_required: z.boolean(),
  }),
});
export const textPageSchema = z.object({
  kind: z.literal('text'),
  value: z.object({
    text: z.string(),
    offset: integer,
    next_offset: integer.nullable(),
    revision: z.string(),
    source_truncated: z.boolean(),
  }),
});

export const taskStateSchema = z.enum([
  'ready',
  'running',
  'waiting_approval',
  'waiting_child',
  'completed',
  'failed',
  'cancelled',
  'interrupted',
  'limited',
]);
export const taskSchema = z.object({
  id: z.string().uuid(),
  parent_id: z.string().uuid().nullable(),
  title: z.string(),
  project_path: z.string(),
  working_path: z.string(),
  model: z.string(),
  permission_mode: z.string(),
  state: taskStateSchema,
  messages: z.array(z.object({ role: z.string(), text: z.string(), at: integer })),
  live_output: z.string(),
  error: z.string().nullable(),
  approvals: z.array(z.object({ id: z.string(), tool: z.string(), input: z.unknown() })),
  updated_at: integer,
  resume_at: integer.nullable().optional(),
  auto_resume_on_limit: z.boolean().optional(),
});
export const taskSnapshotSchema = z.object({
  can_control: z.boolean(),
  revision: integer.optional(),
  tasks: z.array(taskSchema),
});
export const hostSchema = z.object({
  id: z.string().uuid(),
  name: z.string().min(1),
  origin: z.string(),
  authority: z.enum(['desktop', 'tasks']),
  token: z.string().min(1),
  expiresAt: integer.optional(),
});
export type Host = Readonly<z.infer<typeof hostSchema>>;
export type Pairing = Readonly<{ origin: string; secret: string }>;
export type Session = z.infer<typeof sessionSchema>;
export type NativeCommand = z.infer<typeof commandSchema>;
export type NativeAction = z.infer<typeof nativeActionSchema>;
export type Receipt = z.infer<typeof receiptSchema>;
export type Thread = z.infer<typeof threadSchema>;
export type ThreadSnapshot = z.infer<typeof threadSnapshotSchema>['value'];
export type TextPage = z.infer<typeof textPageSchema>['value'];
export type Task = z.infer<typeof taskSchema>;
export type TaskSnapshot = z.infer<typeof taskSnapshotSchema>;
export type Permission = z.infer<typeof permissionSchema>;
export const running = (state: Task['state']): boolean =>
  ['ready', 'running', 'waiting_approval', 'waiting_child'].includes(state);

export function parse<T>(schema: z.ZodType<T>, value: unknown): Result<T> {
  const result = schema.safeParse(value);
  return result.success
    ? success(result.data)
    : failure(
        'invalid',
        'The host returned an incompatible response. Update Bruhs on both devices.',
      );
}

export function originOf(value: string, development = false): Result<string> {
  try {
    const url = new URL(value.trim());
    const localHttp =
      development &&
      url.protocol === 'http:' &&
      ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
    if (
      (url.protocol !== 'https:' && !localHttp) ||
      url.username ||
      url.password ||
      url.search ||
      url.hash ||
      url.pathname !== '/'
    ) {
      return failure(
        'invalid',
        'Use a trusted HTTPS host address without a path, credentials, query or fragment.',
      );
    }
    return success(url.origin);
  } catch {
    return failure('invalid', 'Enter a complete HTTPS host address.');
  }
}

export function parsePairing(value: string, development = false): Result<Pairing> {
  try {
    let url = new URL(value.trim());
    if (url.protocol === 'bruhs:') {
      if (
        url.hostname !== 'pair' ||
        url.username ||
        url.password ||
        url.hash ||
        (url.pathname !== '' && url.pathname !== '/') ||
        url.searchParams.size !== 1 ||
        url.searchParams.getAll('url').length !== 1 ||
        !url.searchParams.get('url')
      )
        return failure(
          'invalid',
          'This Bruhs app link does not contain one unambiguous pairing address.',
        );
      url = new URL(url.searchParams.get('url') ?? '');
    }
    const parameters = new URLSearchParams(url.hash.slice(1));
    const secrets = parameters.getAll('pair');
    if (
      secrets.length !== 1 ||
      parameters.size !== 1 ||
      !/^[A-Za-z0-9_-]{32,512}$/.test(secrets[0] ?? '')
    )
      return failure('invalid', 'Use the complete one-use pairing link shown on your computer.');
    const secret = secrets[0];
    if (!secret) return failure('invalid', 'The pairing secret is missing.');
    url.hash = '';
    const origin = originOf(url.href, development);
    return origin.ok ? success({ origin: origin.value, secret }) : origin;
  } catch {
    return failure('invalid', 'This is not a valid pairing link.');
  }
}

export function validateHost(value: unknown, development = false): Result<Host> {
  const host = parse(hostSchema, value);
  if (!host.ok) return host;
  const origin = originOf(host.value.origin, development);
  if (!origin.ok || origin.value !== host.value.origin)
    return failure('invalid', 'This saved host has an invalid origin. Remove it and pair again.');
  if (/[\s\u0000-\u001f\u007f]/u.test(host.value.token))
    return failure('invalid', 'The host credential is malformed. Pair again.');
  return host;
}

export function requestUrl(host: Host, path: string): Result<string> {
  try {
    const url = new URL(path, host.origin);
    const allowed =
      host.authority === 'desktop'
        ? url.pathname.startsWith('/api/mobile/')
        : ['/api/tasks', '/api/commands', '/api/proposals', '/api/changes'].includes(url.pathname);
    if (
      !path.startsWith('/api/') ||
      path.startsWith('//') ||
      url.origin !== host.origin ||
      !allowed ||
      url.username ||
      url.password ||
      url.hash
    ) {
      return failure('invalid', 'This request is outside the paired host and authority.');
    }
    return success(url.href);
  } catch {
    return failure('invalid', 'The host request address is invalid.');
  }
}

export function draftKey(host: Host, item: string): string {
  return `${host.id}:${host.authority}:${item}`;
}
export function nextCommand(
  id: string,
  sequence: number,
  action: NativeAction,
): Result<NativeCommand> {
  return parse(commandSchema, { id, sequence, action });
}
export function acknowledgeDraft(current: string, sent: string, receipt: Receipt): string {
  return receipt.outcome.status === 'accepted' && current.trim() === sent ? '' : current;
}
