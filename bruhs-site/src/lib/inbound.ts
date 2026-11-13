import PostalMime from "postal-mime";
import { Resend, type Attachment } from "resend";

const RECIPIENTS = new Set(["support@bruhs.ai", "privacy@bruhs.ai"]);
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

type Config = {
  apiKey: string | undefined;
  webhookSecret: string | undefined;
  forwardTo: string | undefined;
};

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (character) => {
    switch (character) {
      case "&": return "&amp;";
      case "<": return "&lt;";
      case ">": return "&gt;";
      case '"': return "&quot;";
      default: return "&#39;";
    }
  });
}

/** Verify a Resend event and forward only the two published bruhs mailboxes. */
export function createInboundHandler(config: Readonly<Config>): (request: Request) => Promise<Response> {
  const resend = config.apiKey ? new Resend(config.apiKey) : null;

  return async (request: Request): Promise<Response> => {
    const respond = (body: string, status = 200): Response => new Response(body, {
      status,
      headers: { "Cache-Control": "no-store" },
    });

    if (request.method !== "POST") {
      return new Response("Method not allowed", { status: 405, headers: { Allow: "POST", "Cache-Control": "no-store" } });
    }
    if (!resend || !config.webhookSecret || !config.forwardTo) {
      console.error("Inbound forwarding requires RESEND_API_KEY, RESEND_WEBHOOK_SECRET and MAIL_FORWARD_TO");
      return respond("Mail forwarding is not configured", 503);
    }

    let event: unknown;
    try {
      event = resend.webhooks.verify({
        payload: await request.text(),
        webhookSecret: config.webhookSecret,
        headers: {
          id: request.headers.get("svix-id") ?? "",
          timestamp: request.headers.get("svix-timestamp") ?? "",
          signature: request.headers.get("svix-signature") ?? "",
        },
      });
    } catch {
      return respond("Invalid signature", 400);
    }

    if (!event || typeof event !== "object" || !("type" in event)) return respond("Invalid email event", 400);
    if (event.type !== "email.received") return respond("Ignored");
    if (!("data" in event) || !event.data || typeof event.data !== "object" || !("email_id" in event.data)) {
      return respond("Invalid email event", 400);
    }
    const emailId = event.data.email_id;
    if (typeof emailId !== "string" || !UUID.test(emailId)) return respond("Invalid email event", 400);

    try {
      const { data: email, error } = await resend.emails.receiving.get(emailId, {}, { signal: request.signal });
      if (error) {
        console.error("Could not retrieve inbound email", { emailId, code: error.name });
        return respond("Could not retrieve email", 502);
      }

      // received_for is the SMTP envelope; To/Cc can be forged or omit Bcc recipients.
      const recipient = email.received_for.find((address) => RECIPIENTS.has(address.toLowerCase()))?.toLowerCase();
      if (!recipient) return respond("Recipient ignored");
      if (Object.keys(email.headers ?? {}).some((name) => name.toLowerCase() === "x-bruhs-forwarded")) {
        return respond("Forwarding loop ignored");
      }
      if (!email.raw?.download_url) {
        console.error("Inbound email has no raw message", { emailId });
        return respond("Original email is unavailable", 502);
      }

      const raw = await fetch(email.raw.download_url, { signal: request.signal });
      if (!raw.ok || !raw.body) {
        console.error("Could not download inbound email", { emailId, status: raw.status });
        return respond("Could not download email", 502);
      }
      const parsed = await PostalMime.parse(raw.body, { attachmentEncoding: "base64" });
      const attachments: Attachment[] = parsed.attachments.map((attachment) => ({
        filename: attachment.filename ?? "attachment",
        content: attachment.content,
        contentType: attachment.mimeType,
        ...(attachment.contentId ? { contentId: attachment.contentId.replace(/^<|>$/g, "") } : {}),
      }));
      const forwarded = `From: ${email.from}\nTo: ${email.received_for.join(", ")}`;
      const { error: sendError } = await resend.emails.send({
        from: `bruhs mail <${recipient}>`,
        to: [config.forwardTo],
        replyTo: email.reply_to?.length ? email.reply_to : email.from,
        subject: `Fwd: ${email.subject || "(no subject)"}`,
        text: `${forwarded}\n\n${parsed.text ?? ""}`,
        ...(parsed.html ? { html: `<p>${escapeHtml(forwarded).replace(/\n/g, "<br>")}</p><hr>${parsed.html}` } : {}),
        attachments,
        headers: { "X-Bruhs-Forwarded": emailId },
      }, { idempotencyKey: `inbound-forward/${emailId}`, signal: request.signal });
      if (sendError) {
        console.error("Could not forward inbound email", { emailId, code: sendError.name });
        return respond("Could not forward email", 502);
      }

      return respond("Forwarded");
    } catch {
      // A non-2xx response keeps the message eligible for Resend's webhook retries.
      console.error("Inbound forwarding failed while downloading or parsing the original message", { emailId });
      return respond("Could not forward email", 502);
    }
  };
}
