# bruhs-site

The landing page at https://bruhs.ai (www redirects with a 308). Static HTML,
CSS and JS in `public/`, with no build step and no functions.

```
npm test           # page + command-bar tests, no install needed (Node 22.18+)
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

## Downloads

There is no backend. The Download buttons link to
`https://github.com/bryantleft/bruhs-ai/releases/latest/download/Bruhs-macOS-arm64.dmg`,
which GitHub resolves to the newest release. Releases are published with
`scripts/release.sh` (see the main repo's CONTRIBUTING.md).

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
