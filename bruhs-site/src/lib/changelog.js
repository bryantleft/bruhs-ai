// Parses the repository's CHANGELOG.md (Keep a Changelog) for /changelog,
// which src/pages/changelog.astro renders at build time. Released versions
// only: `## [Unreleased]` stays in the file until scripts/release.sh ships it.
//
//   node src/lib/changelog.js --check   # exit 1 if CHANGELOG.md won't render

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

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
export function longDate(date) {
  return new Date(`${date}T00:00:00Z`).toLocaleDateString("en-US", {
    year: "numeric",
    month: "long",
    day: "numeric",
    timeZone: "UTC",
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const markdown = readFileSync(new URL("../../../CHANGELOG.md", import.meta.url), "utf8");
  const releases = parseChangelog(markdown);
  if (releases.length === 0) {
    console.error("CHANGELOG.md has no released versions: add a `## [x.y.z] - YYYY-MM-DD` section.");
    process.exit(1);
  }
  console.log(`CHANGELOG.md: ${releases.length} releases, newest v${releases[0]?.version}`);
}
