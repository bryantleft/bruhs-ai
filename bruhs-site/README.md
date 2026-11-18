# bruhs-site

The landing page at https://bruhs.ai (www redirects with a 308). An Astro
site with static output and an inbound email function: one layout (`src/layouts/Base.astro`)
for the head, nav and footer, three pages in `src/pages/`, plain modules in
`src/scripts/`, and fonts, logos and screenshots in `public/`.

```
npm ci             # Node 22.23.4+
npm run dev        # http://localhost:4321
npm test           # builds dist/, then page, CSP, changelog and command-bar tests
npm run typecheck
```

The hero mascot is drawn in dithered braille dots by `src/scripts/braille.js`.

## Brand assets

The bruhs.ai mark is the hero's dot Bruh, 1:1: the same `bruhDots` halftone,
looking straight ahead. Each file is generated at the radius it is shown at, so
it has the grid the site would draw there. The app uses it too: sidebar, web
favicon and Mac icon.

```
node brand/logo.mjs             # site logo/favicon, bruhs-web bruhs.svg/favicon.svg, Mac AppIcon{,-small}.svg
brand/render.sh                 # public/og.png, apple-touch-icon.png and the Mac disk image background, via Chrome
packaging/macos/make-icon.sh    # (repo root) AppIcon.icns, plus the server's tasks icon and the mobile icon
```

## Deploys

Vercel deploys from GitHub (Root Directory
`bruhs-site`): a push to `main` goes to production, a push to any other
branch gets a preview. The `ignoreCommand` in `vercel.json` skips the build
when a commit leaves `bruhs-site/` untouched, so app-only pushes do not
redeploy the site. To deploy by hand, run `npx vercel@latest deploy [--prod]`
from the repository root (the project's Root Directory is resolved from
there).

## Downloads

The Download buttons link to
`https://github.com/bryantleft/bruhs-ai/releases/latest/download/Bruhs-macOS-arm64.dmg`,
which GitHub resolves to the newest release. Releases are published with
`scripts/release.sh` (see the main repo's CONTRIBUTING.md).

## Email

`api/inbound.ts` accepts Resend's signed `email.received` webhook at
`https://bruhs.ai/api/inbound`. It forwards mail addressed to `support@bruhs.ai`
or `privacy@bruhs.ai` to the single `MAIL_FORWARD_TO` address. It uses the SMTP
envelope recipients, including Bcc, rather than trusting To/Cc headers.
Attachments and inline images are parsed from the original message; Reply-To
uses the original Reply-To or sender. Other recipients and forwarding loops
are acknowledged without forwarding.

Set these **production** Vercel environment variables before deploying:

| Variable | Value |
| --- | --- |
| `RESEND_API_KEY` | Full-access key from the bruhs Resend account; sending-only keys cannot retrieve mail |
| `RESEND_WEBHOOK_SECRET` | Signing secret returned when creating the webhook |
| `MAIL_FORWARD_TO` | Owner's inbox, currently `bryantleft@gmail.com` |

Add `bruhs.ai` in Resend with sending and receiving enabled, copy the exact
DNS records returned by Resend into Vercel DNS, and verify the domain. Create
one webhook for `email.received` pointing to `/api/inbound`. Keep production
credentials out of preview deployments. `.env.example` lists the configuration;
`.env.local` and all other environment files remain ignored.

Failures return a non-2xx response so Resend retries. Sends use
`inbound-forward/<email-id>` as their idempotency key, which Resend retains for
24 hours (longer than its automatic webhook retry schedule). A manual replay
after that window can send another copy. Mail remains available in Resend's
Receiving dashboard if forwarding fails; check there and the function logs
for oversized or otherwise rejected messages.

`npm test` covers real webhook signature verification, envelope routing,
attachment content, Reply-To, retry idempotency and failures without sending
real emails. After setup, send a real message to each address and confirm
that the forwarded copies reach the owner's inbox.

## Screenshots

Every picture of the app on the page is a real capture of `bruhs-web`, built
from the committed `HEAD` and filled with fictional demo data
(`capture/seed.mjs`). `capture/validator` checks the seed against the real
`bruhs-models` types before anything renders. To refresh them after a UI
change:

```
capture/recapture.sh --build   # export HEAD, build bruhs-web, capture, write public/shots/
capture/recapture.sh           # reuse the last build
```

It needs trunk, cargo, node, python3, Google Chrome, ImageMagick and cwebp,
and it binds 127.0.0.1:8111 and :9351. `capture/export.sh` alone redoes the
crop and WebP step from the raw PNGs in `/tmp/bruhs-shots`.

## Keeping it honest

- `/changelog` is built from the repository's `CHANGELOG.md`
  (`src/lib/changelog.js` parses it), so a push that only edits
  `CHANGELOG.md` redeploys the site (`ignoreCommand` in `vercel.json`).
- The keyboard map mirrors the defaults in
  `bruhs-models/src/logic/keybindings.rs`. Update both together.
- Captions under each screenshot describe what the app does today. Re-capture
  when a surface changes enough that the picture and the caption disagree.
- The CSP in `vercel.json` allows only same-origin scripts, styles, fonts and
  images, so no inline `style=""` or `<script>` in the HTML. Astro inlines
  small ones by default; `astro.config.mjs` turns that off and
  `test/build.test.ts` fails if any appear in `dist/`.
