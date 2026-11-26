import { z } from 'zod';
import {
  type Host,
  type NativeCommand,
  type Pairing,
  type PendingQuestion,
  type Result,
  failure,
  linkedDraftSchema,
  messageOf,
  parse,
  originOf,
  pairSchema,
  receiptSchema,
  requestUrl,
  sessionSchema,
  taskSnapshotSchema,
  textPageSchema,
  threadSnapshotSchema,
  threadsSchema,
} from './domain.ts';

// Native mobile broker delivery is 15 seconds; the existing browser client
// reserves one additional second for transport and cancellation.
export const REQUEST_MS = 16_000;
export const DESKTOP_RESPONSE_BYTES = 32 * 1024;
// Reuses the companion API's default JSON body budget for a task snapshot.
export const TASK_RESPONSE_BYTES = 2 * 1024 * 1024;
export type WireResponse = Readonly<{ status: number; text: string }>;
export type TextPageRequest = Readonly<{ offset: number; revision?: string }> & (
  | Readonly<{ kind: 'diff' }>
  | Readonly<{ kind: 'message'; message: { index: number; reference: string } }>
  | Readonly<{ kind: 'question-link'; title: string; questionLink: {
    id: string; requestId: string; reviewHash: string; turnGeneration: number;
  } }>
);
export type Transport = (
  url: string,
  init: Readonly<{
    method: 'GET' | 'POST' | 'DELETE';
    token?: string;
    body?: string | Blob;
    contentType?: string;
    signal: AbortSignal;
    maxBytes: number;
  }>,
) => Promise<WireResponse>;

export class HostClient {
  readonly host: Host;
  private readonly transport: Transport;
  private readonly signal?: AbortSignal;
  constructor(host: Host, transport: Transport, signal?: AbortSignal) {
    this.host = host;
    this.transport = transport;
    this.signal = signal;
  }

  async request<T>(
    path: string,
    schema: z.ZodType<T>,
    body?: unknown,
    signal?: AbortSignal,
  ): Promise<Result<T>> {
    const target = requestUrl(this.host, path);
    if (!target.ok) return target;
    return request(
      this.transport,
      target.value,
      schema,
      this.host.authority === 'desktop' ? DESKTOP_RESPONSE_BYTES : TASK_RESPONSE_BYTES,
      this.host.token,
      body,
      signal ?? this.signal,
    );
  }

  session() {
    return this.request('/api/mobile/session', sessionSchema);
  }
  threads(offset = 0) {
    return this.request(`/api/mobile/threads?offset=${offset}`, threadsSchema);
  }
  async linkedDraft(id: string, signal?: AbortSignal) {
    const query = new URLSearchParams({ link_id: id });
    const result = await this.request(`/api/mobile/linked-draft?${query}`, linkedDraftSchema, undefined, signal);
    if (result.ok && result.value.value.link_id !== id) {
      return failure('conflict', 'The computer returned a different linked draft.');
    }
    return result;
  }
  async thread(id: string, before?: number) {
    const q = new URLSearchParams({ thread_id: id });
    if (before !== undefined) q.set('before', String(before));
    const result = await this.request(`/api/mobile/thread?${q}`, threadSnapshotSchema);
    if (result.ok && result.value.value.thread.id !== id) {
      return failure('conflict', 'The computer returned a different conversation than requested.');
    }
    return result;
  }
  text(id: string, page: TextPageRequest) {
    const q = new URLSearchParams({ thread_id: id, offset: String(page.offset) });
    if (page.revision) q.set('revision', page.revision);
    if (page.kind === 'message') {
      q.set('index', String(page.message.index));
      q.set('reference', page.message.reference);
    } else if (page.kind === 'question-link') {
      const link = page.questionLink;
      q.set('link', link.id);
      q.set('request_id', link.requestId);
      q.set('review_hash', link.reviewHash);
      q.set('turn_generation', String(link.turnGeneration));
    }
    return this.request(`/api/mobile/${page.kind}?${q}`, textPageSchema);
  }
  command(command: NativeCommand) {
    return this.request('/api/mobile/command', receiptSchema, command);
  }
  questionFile(
    thread: string,
    requestReview: PendingQuestion,
    position: number,
    id: string,
    file?: Readonly<{ name: string; body: Blob }>,
  ) {
    const q = new URLSearchParams({
      thread_id: thread,
      turn_generation: String(requestReview.turn_generation),
      request_id: requestReview.request_id,
      review_hash: requestReview.review_hash,
      question: String(position),
      upload_id: id,
    });
    if (file) q.set('name', file.name);
    const target = requestUrl(this.host, `/api/mobile/question-file?${q}`);
    if (!target.ok) return Promise.resolve(target);
    return request(
      this.transport, target.value, z.object({ ok: z.literal(true) }), DESKTOP_RESPONSE_BYTES,
      this.host.token, undefined, this.signal, file ? { body: file.body } : {},
    );
  }
  disconnect() {
    return this.request('/api/mobile/disconnect', z.object({ disconnected: z.literal(true) }), {});
  }
  tasks() {
    return this.request('/api/tasks', taskSnapshotSchema);
  }
  taskCommand(command: unknown) {
    return this.request('/api/commands', taskSnapshotSchema, command);
  }
  proposal(text: string) {
    return this.request(
      '/api/proposals',
      z.object({ id: z.string(), task_id: z.string(), text: z.string(), at: z.number() }),
      { text },
    );
  }
}

export async function pairDesktop(
  transport: Transport,
  pairing: Pairing,
  name: string,
  development = false,
): Promise<Result<z.infer<typeof pairSchema>>> {
  const origin = originOf(pairing.origin, development);
  if (!origin.ok || origin.value !== pairing.origin)
    return failure('invalid', 'Review the trusted host address before pairing.');
  return request(
    transport,
    `${pairing.origin}/api/mobile/pair`,
    pairSchema,
    DESKTOP_RESPONSE_BYTES,
    undefined,
    { secret: pairing.secret, name },
  );
}

async function request<T>(
  transport: Transport,
  url: string,
  schema: z.ZodType<T>,
  maxBytes: number,
  token?: string,
  body?: unknown,
  outer?: AbortSignal,
  file?: Readonly<{ body?: Blob }>,
): Promise<Result<T>> {
  const controller = new AbortController();
  const abort = () => controller.abort();
  outer?.addEventListener('abort', abort, { once: true });
  if (outer?.aborted) controller.abort();
  const timer = setTimeout(abort, file?.body ? 200_000 : REQUEST_MS);
  const method = file ? file.body ? 'POST' : 'DELETE' : body === undefined ? 'GET' : 'POST';
  try {
    const payload = file?.body ?? (body === undefined ? undefined : JSON.stringify(body));
    const response = await transport(url, {
      method,
      token,
      body: payload,
      contentType: file?.body ? 'application/octet-stream' : payload === undefined ? undefined : 'application/json',
      signal: controller.signal,
      maxBytes,
    });
    const delivery =
      method === 'GET'
        ? undefined
        : response.status >= 400 && response.status < 500 && response.status !== 408
          ? 'rejected'
          : 'uncertain';
    let value: unknown;
    try {
      value = JSON.parse(response.text);
    } catch {
      return failure(
        response.status === 401
          ? 'unauthorized'
          : response.status === 403
            ? 'forbidden'
            : 'invalid',
        'The host did not return readable JSON. Check the address and proxy configuration.',
        delivery,
      );
    }
    if (response.status < 200 || response.status >= 300) {
      const detail = z.object({ error: z.string() }).safeParse(value);
      const message = detail.success
        ? detail.data.error
        : `The host rejected this request (${response.status}).`;
      return failure(
        response.status === 401
          ? 'unauthorized'
          : response.status === 403
            ? 'forbidden'
            : response.status === 409
              ? 'conflict'
              : 'invalid',
        message,
        delivery,
      );
    }
    const parsed = parse(schema, value);
    return parsed.ok ? parsed : failure(parsed.error.kind, parsed.error.message, delivery);
  } catch (error) {
    return failure(
      'network',
      controller.signal.aborted
        ? 'The host did not answer before its delivery deadline. Reconnect and check the current state.'
        : `Could not reach the paired host. ${messageOf(error)}`,
      method === 'GET' ? undefined : 'uncertain',
    );
  } finally {
    clearTimeout(timer);
    outer?.removeEventListener('abort', abort);
  }
}
