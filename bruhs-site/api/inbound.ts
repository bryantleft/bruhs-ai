import { createInboundHandler } from "../src/lib/inbound.js";

export default {
  fetch: createInboundHandler({
    apiKey: process.env.RESEND_API_KEY,
    webhookSecret: process.env.RESEND_WEBHOOK_SECRET,
    forwardTo: process.env.MAIL_FORWARD_TO,
  }),
};
