# bruhs-site

The landing page at https://bruhs.ai (www redirects with a 308) and its
waitlist endpoint. Static HTML, CSS and
JS in `public/` (no build step), plus one Vercel function in `api/`.

```
npm test           # handler + command-bar tests, no install needed (Node 22.18+)
npm ci && npm run typecheck
```

## Deploys

Vercel deploys from GitHub (Root Directory
`bruhs-site`): a push to `main` goes to production, a push to any other
branch gets a preview. The `ignoreCommand` in `vercel.json` skips the build
when a commit leaves `bruhs-site/` untouched, so app-only pushes do not
redeploy the site. To deploy by hand, run `npx vercel@latest deploy [--prod]`
from the repository root (the project's Root Directory is resolved from
there).

## Waitlist storage

`api/waitlist.ts` writes to Upstash Redis through its REST API. It reads
`KV_REST_API_URL` and `KV_REST_API_TOKEN`, which the Upstash integration in
the Vercel dashboard sets when the database is linked to `bruhs-ai-site`.
Until they exist the endpoint answers 503 `storage_unconfigured`.

- Signups: hash `waitlist:entries`, field = lowercased email, value =
  `{ joinedAt, consent }`. Export with `HGETALL waitlist:entries`.
- Rate limit: `waitlist:rate:<sha256(ip)>`, 10 per hour, expires.
- Changing the wording under the form? Bump `CONSENT_VERSION`.

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

- `public/changelog.html` is written by hand from `git log`, one `<article>`
  per Monday-to-Sunday week, newest first, and only the newest carries
  `badge-current`. `test/page.test.ts` checks the ids, order and badge.
- The keyboard map mirrors the defaults in
  `bruhs-models/src/logic/keybindings.rs`. Update both together.
- Captions under each screenshot describe what the app does today. Re-capture
  when a surface changes enough that the picture and the caption disagree.
- The CSP in `vercel.json` allows only same-origin scripts, styles, fonts and
  images, so no inline `style=""` or `<script>` in the HTML.
