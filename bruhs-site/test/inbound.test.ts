import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { describe, it, type TestContext } from "node:test";
import { createInboundHandler } from "../src/lib/inbound.ts";

const EMAIL_ID = "a1b2c3d4-1111-2222-3333-444455556666";
const SECRET = Buffer.from("bruhs-webhook-signature-test").toString("base64");
const handler = createInboundHandler({
  apiKey: "re_test",
  webhookSecret: `whsec_${SECRET}`,
  forwardTo: "owner@example.com",
});

function request(event: unknown = { type: "email.received", data: { email_id: EMAIL_ID } }, timestamp = Math.floor(Date.now() / 1000)): Request {
  const payload = JSON.stringify(event);
  const id = "msg_inbound_test";
  const signature = createHmac("sha256", Buffer.from(SECRET, "base64")).update(`${id}.${timestamp}.${payload}`).digest("base64");
  return new Request("https://bruhs.ai/api/inbound", {
    method: "POST",
    body: payload,
    headers: {
      "Content-Type": "application/json",
      "svix-id": id,
      "svix-timestamp": String(timestamp),
      "svix-signature": `v1,${signature}`,
    },
  });
}

const rawEmail = [
  "From: Sender <sender@example.com>",
  "To: support@bruhs.ai",
  "Subject: Help with bruhs",
  'Content-Type: multipart/mixed; boundary="outer"',
  "",
  "--outer",
  'Content-Type: multipart/alternative; boundary="inner"',
  "",
  "--inner",
  "Content-Type: text/plain; charset=utf-8",
  "",
  "Please help.",
  "--inner",
  "Content-Type: text/html; charset=utf-8",
  "",
  '<p>Please help.</p><img src="cid:sample">',
  "--inner--",
  "--outer",
  "Content-Type: image/png",
  "Content-Transfer-Encoding: base64",
  'Content-Disposition: inline; filename="sample.png"',
  "Content-ID: <sample>",
  "",
  "aGVsbG8=",
  "--outer--",
].join("\r\n");

const email = {
  id: EMAIL_ID,
  from: "Sender <sender@example.com>",
  to: ["support@bruhs.ai"],
  received_for: ["support@bruhs.ai"],
  reply_to: ["replies@example.com"],
  subject: "Help with bruhs",
  headers: {},
  raw: { download_url: "https://storage.example.com/original.eml" },
};

function mockApi(t: TestContext, received: object = email, sendStatus = 200): { url: string; options: RequestInit | undefined }[] {
  const calls: { url: string; options: RequestInit | undefined }[] = [];
  t.mock.method(globalThis, "fetch", async (url: string | URL | Request, options?: RequestInit): Promise<Response> => {
    const address = String(url);
    calls.push({ url: address, options });
    if (address === `https://api.resend.com/emails/receiving/${EMAIL_ID}`) return Response.json(received);
    if (address === email.raw.download_url) return new Response(rawEmail);
    if (address === "https://api.resend.com/emails") {
      return sendStatus === 200 ? Response.json({ id: "forwarded-id" }) : Response.json({ name: "rate_limit_exceeded", message: "Try later" }, { status: sendStatus });
    }
    throw new Error(`Unexpected API request: ${address}`);
  });
  return calls;
}

describe("inbound email forwarding", () => {
  it("rejects unsigned, altered and expired requests before contacting Resend", async (t) => {
    const calls = mockApi(t);
    const unsigned = new Request("https://bruhs.ai/api/inbound", { method: "POST", body: "{}" });
    const altered = request();
    altered.headers.set("svix-signature", "v1,invalid");
    for (const input of [unsigned, altered, request(undefined, 1)]) assert.equal((await handler(input)).status, 400);
    assert.equal(calls.length, 0);
  });

  it("requires POST and configured secrets", async () => {
    const response = await handler(new Request("https://bruhs.ai/api/inbound"));
    assert.equal(response.status, 405);
    assert.equal(response.headers.get("allow"), "POST");
    const unconfigured = createInboundHandler({ apiKey: undefined, webhookSecret: undefined, forwardTo: undefined });
    assert.equal((await unconfigured(request())).status, 503);
  });

  it("validates signed payloads and ignores unrelated events", async (t) => {
    const calls = mockApi(t);
    for (const payload of [null, {}, { type: "email.received" }, { type: "email.received", data: { email_id: "../domains" } }]) {
      assert.equal((await handler(request(payload))).status, 400);
    }
    assert.equal((await handler(request({ type: "email.sent" }))).status, 200);
    assert.equal(calls.length, 0);
  });

  it("forwards the original body and inline attachment once to the fixed owner, with Reply-To", async (t) => {
    const calls = mockApi(t);
    const response = await handler(request());
    assert.equal(response.status, 200);
    assert.equal(response.headers.get("cache-control"), "no-store");
    assert.equal(calls.length, 3);
    const sent = calls.find((call) => call.url === "https://api.resend.com/emails");
    assert.ok(sent?.options);
    const body = JSON.parse(String(sent.options.body));
    assert.deepEqual(body.to, ["owner@example.com"]);
    assert.equal(body.from, "bruhs mail <support@bruhs.ai>");
    assert.deepEqual(body.reply_to, ["replies@example.com"]);
    assert.match(body.text, /Please help\./);
    assert.match(body.html, /Sender &lt;sender@example.com&gt;/);
    assert.match(body.html, /src="cid:sample"/);
    assert.deepEqual(body.attachments, [{ filename: "sample.png", content: "aGVsbG8=", content_type: "image/png", content_id: "sample" }]);
    assert.equal(new Headers(sent.options.headers).get("idempotency-key"), `inbound-forward/${EMAIL_ID}`);
  });

  it("uses envelope recipients for privacy mail even when it is Bcc'd", async (t) => {
    const calls = mockApi(t, { ...email, to: ["someone@example.com"], received_for: ["PRIVACY@bruhs.ai", "support@bruhs.ai"], reply_to: [] });
    assert.equal((await handler(request())).status, 200);
    const sends = calls.filter((call) => call.url === "https://api.resend.com/emails");
    assert.equal(sends.length, 1);
    const body = JSON.parse(String(sends[0]?.options?.body));
    assert.equal(body.from, "bruhs mail <privacy@bruhs.ai>");
    assert.equal(body.reply_to, email.from);
  });

  it("ignores forged To headers and unknown envelope recipients", async (t) => {
    const calls = mockApi(t, { ...email, received_for: ["unknown@bruhs.ai"] });
    assert.equal((await handler(request())).status, 200);
    assert.equal(calls.length, 1);
  });

  it("stops mail that has already passed through the forwarder", async (t) => {
    const calls = mockApi(t, { ...email, headers: { "X-Bruhs-Forwarded": EMAIL_ID } });
    assert.equal((await handler(request())).status, 200);
    assert.equal(calls.length, 1);
  });

  it("keeps failures retryable and retry sends identical", async (t) => {
    const calls = mockApi(t, email, 429);
    for (let i = 0; i < 2; i++) assert.equal((await handler(request())).status, 502);
    const sends = calls.filter((call) => call.url === "https://api.resend.com/emails");
    assert.equal(sends.length, 2);
    assert.deepEqual(sends[0]?.options?.body, sends[1]?.options?.body);
    assert.equal(new Headers(sends[0]?.options?.headers).get("idempotency-key"), new Headers(sends[1]?.options?.headers).get("idempotency-key"));
  });

  it("does not acknowledge an email whose original cannot be retrieved", async (t) => {
    mockApi(t, { ...email, raw: null });
    assert.equal((await handler(request())).status, 502);
  });
});
