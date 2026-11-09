// POST /api/waitlist: adds one email to the launch waitlist in Upstash Redis.
//
// Accepts JSON (the page's fetch) or a urlencoded form (the no-JS fallback,
// answered with a 303 back to the page). The only data kept per signup is
// the address, when it joined, and which consent wording it saw.

/** One Redis command in Upstash REST form, e.g. `["INCR", "key"]`. */
export type RedisCommand = readonly (string | number)[];
export type RedisReply = { readonly result: unknown } | { readonly error: string };
export type RedisPipeline = (commands: readonly RedisCommand[]) => Promise<readonly RedisReply[]>;

export interface WaitlistDeps {
  /** `null` when the Upstash env vars are missing; the handler answers 503. */
  readonly pipeline: RedisPipeline | null;
  readonly now: () => Date;
}

// The page maps each code to visitor copy (public/messages.js); the
// `message` here is for whoever reads the API response or the logs.
export const FAILURE_CODES = [
  "invalid_body",
  "invalid_email",
  "payload_too_large",
  "unsupported_media_type",
  "rate_limited",
  "storage_unconfigured",
  "storage_failed",
] as const;
type FailureCode = (typeof FAILURE_CODES)[number];

type Outcome =
  | { readonly kind: "joined" }
  | { readonly kind: "failed"; readonly status: number; readonly code: FailureCode; readonly message: string };

type Signup = { readonly email: unknown; readonly honeypot: unknown };

const ENTRIES_KEY = "waitlist:entries";
// Bump when the wording under the form changes, so each stored signup says
// which promise it agreed to.
const CONSENT_VERSION = "launch-emails-v1";

// RFC 5321 caps a forward path at 256 octets including the angle brackets.
const MAX_EMAIL_LENGTH = 254;
// The largest real body is a 254-char address, percent-encoded (up to 3x)
// in the form fallback, plus field names: under 900 bytes.
const MAX_BODY_BYTES = 2048;

// A person signs up once, maybe again after a typo. Ten an hour per IP still
// covers a shared office or household NAT, and caps a single-IP script at
// 240 signups a day.
export const RATE_LIMIT_PER_HOUR = 10;
const RATE_WINDOW_SECONDS = 3600;

const EMAIL_SHAPE = /^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/;

const ENV_URL = "KV_REST_API_URL";
const ENV_TOKEN = "KV_REST_API_TOKEN";

export async function handleWaitlist(request: Request, deps: WaitlistDeps): Promise<Response> {
  const wantsHtml = isFormPost(request);
  const outcome = await run(request, deps);
  if (outcome.kind === "failed" && outcome.status >= 500) {
    console.error(JSON.stringify({ event: "waitlist_failed", code: outcome.code, message: outcome.message }));
  }
  return wantsHtml ? redirectResponse(outcome) : jsonResponse(outcome);
}

async function run(request: Request, deps: WaitlistDeps): Promise<Outcome> {
  const body = await readSignup(request);
  if ("kind" in body) return body;

  // Bots fill every field; people never see this one. Answer like success so
  // the bot learns nothing.
  if (typeof body.honeypot === "string" && body.honeypot !== "") return { kind: "joined" };

  const email = normalizeEmail(body.email);
  if (email === null) {
    return failed(400, "invalid_email", "Enter a full email address, like you@domain.com.");
  }

  if (deps.pipeline === null) {
    return failed(
      503,
      "storage_unconfigured",
      `Waitlist storage is not configured: set ${ENV_URL} and ${ENV_TOKEN} (add the Upstash Redis integration in Vercel).`,
    );
  }

  try {
    const rateKey = `waitlist:rate:${await sha256Hex(clientIp(request))}`;
    const [hits] = await exec(deps.pipeline, [
      ["INCR", rateKey],
      ["EXPIRE", rateKey, RATE_WINDOW_SECONDS, "NX"],
    ]);
    if (typeof hits === "number" && hits > RATE_LIMIT_PER_HOUR) {
      return failed(429, "rate_limited", "Too many signups from this network. Try again in an hour.");
    }

    const record = JSON.stringify({ joinedAt: deps.now().toISOString(), consent: CONSENT_VERSION });
    // HSETNX keeps the first join time. A repeat signup gets the same answer
    // as a new one, so the endpoint never confirms who is on the list.
    await exec(deps.pipeline, [["HSETNX", ENTRIES_KEY, email, record]]);
    return { kind: "joined" };
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    return failed(502, "storage_failed", `Could not save the signup: ${detail}`);
  }
}

async function readSignup(request: Request): Promise<Signup | Extract<Outcome, { kind: "failed" }>> {
  const declared = Number(request.headers.get("content-length") ?? "0");
  if (declared > MAX_BODY_BYTES) return tooLarge();
  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > MAX_BODY_BYTES) return tooLarge();

  const type = (request.headers.get("content-type") ?? "").split(";")[0]?.trim().toLowerCase();
  if (type === "application/json") {
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      return failed(400, "invalid_body", "The request body is not valid JSON.");
    }
    if (typeof parsed !== "object" || parsed === null) {
      return failed(400, "invalid_body", 'Send a JSON object like {"email": "you@domain.com"}.');
    }
    const fields = parsed as Record<string, unknown>;
    return { email: fields.email, honeypot: fields.website };
  }
  if (type === "application/x-www-form-urlencoded") {
    const fields = new URLSearchParams(text);
    return { email: fields.get("email"), honeypot: fields.get("website") };
  }
  return failed(415, "unsupported_media_type", "Send JSON or a urlencoded form.");
}

function normalizeEmail(raw: unknown): string | null {
  if (typeof raw !== "string") return null;
  const email = raw.trim().toLowerCase();
  if (email.length > MAX_EMAIL_LENGTH || !EMAIL_SHAPE.test(email)) return null;
  return email;
}

/** Runs the commands and returns their results, throwing on any Redis error reply. */
async function exec(pipeline: RedisPipeline, commands: readonly RedisCommand[]): Promise<unknown[]> {
  const replies = await pipeline(commands);
  if (replies.length !== commands.length) {
    throw new Error(`Redis returned ${replies.length} replies for ${commands.length} commands`);
  }
  return replies.map((reply, index) => {
    if ("error" in reply) throw new Error(`Redis ${String(commands[index]?.[0])} failed: ${reply.error}`);
    return reply.result;
  });
}

// Vercel sets x-real-ip to the connecting client. The hash keeps raw IPs out
// of Redis; the keys also expire after the rate window.
function clientIp(request: Request): string {
  const forwarded = request.headers.get("x-forwarded-for")?.split(",")[0]?.trim();
  return request.headers.get("x-real-ip") ?? (forwarded || "unknown");
}

async function sha256Hex(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function isFormPost(request: Request): boolean {
  const type = request.headers.get("content-type") ?? "";
  const accept = request.headers.get("accept") ?? "";
  return type.startsWith("application/x-www-form-urlencoded") && accept.includes("text/html");
}

function jsonResponse(outcome: Outcome): Response {
  const headers = { "cache-control": "no-store" };
  if (outcome.kind === "joined") return Response.json({ status: "joined" }, { headers });
  return Response.json(
    { error: outcome.code, message: outcome.message },
    {
      status: outcome.status,
      headers: outcome.code === "rate_limited" ? { ...headers, "retry-after": String(RATE_WINDOW_SECONDS) } : headers,
    },
  );
}

// Without JS the page can't render a result, so success goes to its own page
// and failures come back with a code the page script explains, if it runs.
function redirectResponse(outcome: Outcome): Response {
  const location = outcome.kind === "joined" ? "/joined" : `/?waitlist=${outcome.code}#join`;
  return new Response(null, { status: 303, headers: { location, "cache-control": "no-store" } });
}

function failed(status: number, code: FailureCode, message: string): Extract<Outcome, { kind: "failed" }> {
  return { kind: "failed", status, code, message };
}

function tooLarge(): Extract<Outcome, { kind: "failed" }> {
  return failed(413, "payload_too_large", `The request body is over ${MAX_BODY_BYTES} bytes.`);
}

/** Upstash REST adapter: one HTTP round trip per pipeline. */
export function upstashPipeline(
  baseUrl: string,
  token: string,
  fetchImpl: typeof fetch = fetch,
): RedisPipeline {
  const endpoint = `${baseUrl.replace(/\/+$/, "")}/pipeline`;
  return async (commands) => {
    const response = await fetchImpl(endpoint, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify(commands),
    });
    if (!response.ok) {
      throw new Error(`Upstash responded ${response.status}: ${(await response.text()).slice(0, 200)}`);
    }
    return (await response.json()) as RedisReply[];
  };
}

// The Vercel Upstash integration sets KV_REST_API_*; a direct Upstash
// database exposes UPSTASH_REDIS_REST_*. Read once per cold start.
const storageUrl = process.env[ENV_URL] ?? process.env.UPSTASH_REDIS_REST_URL;
const storageToken = process.env[ENV_TOKEN] ?? process.env.UPSTASH_REDIS_REST_TOKEN;
const productionPipeline = storageUrl && storageToken ? upstashPipeline(storageUrl, storageToken) : null;

export function POST(request: Request): Promise<Response> {
  return handleWaitlist(request, { pipeline: productionPipeline, now: () => new Date() });
}
