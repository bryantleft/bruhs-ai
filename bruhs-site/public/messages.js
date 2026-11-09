// What a visitor sees when a signup fails, keyed by the endpoint's error
// code. The endpoint's own `message` is written for us, not for visitors.

export const FALLBACK_MESSAGE = "We couldn't save your email just now. Try again in a few minutes.";

const RELOAD = "We couldn't read that signup. Reload the page and try again.";

/** @type {Readonly<Record<string, string>>} */
const MESSAGES = {
  invalid_email: "Enter a full email address, like you@domain.com.",
  rate_limited: "Too many signups from this network. Try again in an hour.",
  invalid_body: RELOAD,
  payload_too_large: RELOAD,
  unsupported_media_type: RELOAD,
  storage_unconfigured: "Signups are paused for a moment. Try again in a few minutes.",
  storage_failed: "We couldn't save your email just now. Try again in a minute.",
};

/**
 * @param {string | null | undefined} code
 * @returns {string}
 */
export function waitlistMessage(code) {
  return (code && Object.hasOwn(MESSAGES, code) && MESSAGES[code]) || FALLBACK_MESSAGE;
}
