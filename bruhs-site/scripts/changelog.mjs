// Renders the repository's CHANGELOG.md into public/changelog.html, between
// the `<!-- changelog:start -->` and `<!-- changelog:end -->` markers.
//
//   node scripts/changelog.mjs           # rewrite the page
//   node scripts/changelog.mjs --check   # exit 1 if the page is out of date
//
// Released versions only: `## [Unreleased]` stays in the file until
// scripts/release.sh ships it. Each release becomes one <article> with an
// anchor (`#v0.1.1`), its date, and Added / Changed / Fixed lists.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const REPO = "https://github.com/bryantleft/bruhs-ai";
const START = "<!-- changelog:start -->";
const END = "<!-- changelog:end -->";
const KINDS = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"];

/**
 * @typedef {{ kind: string, items: string[] }} Section
 * @typedef {{ version: string, date: string, summary: string, sections: Section[] }} Release
 */

/**
 * The released versions in a Keep a Changelog file, newest first.
 * @param {string} markdown
 * @returns {Release[]}
 */
export function parseChangelog(markdown) {
  /** @type {Release[]} */
  const releases = [];
  /** @type {Release | null} */
  let release = null;
  /** @type {Section | null} */
  let section = null;
  /** @type {string[]} */
  let summary = [];
  let skipping = true;

  for (const raw of markdown.split("\n")) {
    const line = raw.trimEnd();
    const heading = /^## \[([^\]]+)\](?: - (\d{4}-\d{2}-\d{2}))?\s*$/.exec(line);
    if (heading) {
      const version = heading[1] ?? "";
      skipping = version === "Unreleased" || !/^\d+\.\d+\.\d+/.test(version);
      if (!skipping && !heading[2]) throw new Error(`## [${version}] has no date`);
      release = skipping ? null : { version, date: heading[2] ?? "", summary: "", sections: [] };
      if (release) releases.push(release);
      section = null;
      summary = [];
      continue;
    }
    if (/^\[[^\]]+\]: /.test(line)) continue; // link references
    if (skipping || !release) continue;

    const kind = /^### (.+)$/.exec(line);
    if (kind) {
      const name = (kind[1] ?? "").trim();
      if (!KINDS.includes(name)) throw new Error(`${release.version}: unknown section "${name}"`);
      section = { kind: name, items: [] };
      release.sections.push(section);
      continue;
    }
    const bullet = /^- (.*)$/.exec(line);
    if (bullet && section) {
      section.items.push(bullet[1] ?? "");
    } else if (/^\s+\S/.test(line) && section && section.items.length) {
      section.items[section.items.length - 1] += ` ${line.trim()}`;
    } else if (line && !section) {
      summary.push(line.trim());
      release.summary = summary.join(" ");
    }
  }
  return releases;
}

/** @param {string} text */
function escapeHtml(text) {
  return text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
}

/**
 * Inline Markdown: `code`, **bold** and [links](https://…). Everything else
 * is escaped text.
 * @param {string} text
 */
export function inline(text) {
  return text
    .split(/(`[^`]+`)/)
    .map((part) => {
      if (part.startsWith("`") && part.endsWith("`") && part.length > 1) {
        return `<code>${escapeHtml(part.slice(1, -1))}</code>`;
      }
      return escapeHtml(part)
        .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
        .replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+|\/[^)\s]*)\)/g, '<a href="$2">$1</a>');
    })
    .join("");
}

/** @param {string} date `2026-11-09` */
function longDate(date) {
  return new Date(`${date}T00:00:00Z`).toLocaleDateString("en-US", {
    year: "numeric",
    month: "long",
    day: "numeric",
    timeZone: "UTC",
  });
}

/**
 * @param {Release[]} releases
 * @returns {string}
 */
export function renderReleases(releases) {
  const pad = "        ";
  return releases
    .map((release, index) => {
      const id = `v${release.version}`;
      const badges = index === 0 ? `\n${pad}    <p class="badges"><span class="badge badge-current">Current</span></p>` : "";
      const summary = release.summary
        ? `\n${pad}    <p class="release-summary">${inline(release.summary)}</p>`
        : "";
      const sections = release.sections
        .map((section) => {
          const sid = `${id}-${section.kind.toLowerCase()}`;
          // One <span> per item: the <li> is a two-column grid (dash, text), so
          // inline <code> must not become a grid cell of its own.
          const items = section.items.map((item) => `${pad}        <li><span>${inline(item)}</span></li>`).join("\n");
          return [
            `${pad}    <section class="changes" aria-labelledby="${sid}">`,
            `${pad}      <h3 class="change-kind kind-${section.kind.toLowerCase()}" id="${sid}">${section.kind}</h3>`,
            `${pad}      <ul>`,
            items,
            `${pad}      </ul>`,
            `${pad}    </section>`,
          ].join("\n");
        })
        .join("\n");
      const download = index === 0 ? `<a href="/#get">Download latest</a>` : "";
      return [
        `${pad}<article class="release" id="${id}">`,
        `${pad}  <header class="release-meta">`,
        `${pad}    <p class="release-week"><time datetime="${release.date}">${longDate(release.date)}</time></p>`,
        `${pad}    <h2 class="release-title"><a href="#${id}">${id}</a></h2>${badges}`,
        `${pad}  </header>`,
        `${pad}  <div class="release-body">${summary}`,
        sections,
        `${pad}    <p class="release-links">${download}<a href="${REPO}/releases/tag/${id}">Release details <span aria-hidden="true">→</span></a></p>`,
        `${pad}  </div>`,
        `${pad}</article>`,
      ].join("\n");
    })
    .join("\n\n");
}

/**
 * The page with its changelog region replaced.
 * @param {string} page
 * @param {string} markdown
 */
export function renderPage(page, markdown) {
  const start = page.indexOf(START);
  const end = page.indexOf(END);
  if (start < 0 || end < start) throw new Error("changelog.html has no changelog markers");
  return `${page.slice(0, start + START.length)}\n${renderReleases(parseChangelog(markdown))}\n        ${page.slice(end)}`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const changelogPath = new URL("../../CHANGELOG.md", import.meta.url);
  const pagePath = new URL("../public/changelog.html", import.meta.url);
  const page = readFileSync(pagePath, "utf8");
  const next = renderPage(page, readFileSync(changelogPath, "utf8"));
  if (process.argv.includes("--check")) {
    if (next !== page) {
      console.error("public/changelog.html is out of date: run `npm run changelog`.");
      process.exit(1);
    }
  } else if (next !== page) {
    writeFileSync(pagePath, next);
    console.log("Updated public/changelog.html");
  }
}
