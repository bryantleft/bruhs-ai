import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

import { inline, parseChangelog } from "../src/lib/changelog.js";

const CHANGELOG = readFileSync(new URL("../../CHANGELOG.md", import.meta.url), "utf8");

describe("CHANGELOG.md", () => {
  it("dates every released version", () => {
    for (const release of parseChangelog(CHANGELOG)) {
      assert.match(release.date, /^\d{4}-\d{2}-\d{2}$/, `v${release.version}`);
      assert.ok(release.sections.length > 0, `v${release.version} lists no changes`);
    }
  });
});

describe("parseChangelog", () => {
  const sample = [
    "# Changelog",
    "",
    "## [Unreleased]",
    "",
    "### Added",
    "",
    "- Not shipped yet.",
    "",
    "## [1.2.0] - 2026-12-01",
    "",
    "A summary that wraps",
    "onto two lines.",
    "",
    "### Fixed",
    "",
    "- A fix that wraps",
    "  onto a second line.",
    "- Another fix.",
    "",
    "[1.2.0]: https://example.com",
  ].join("\n");

  it("skips Unreleased and joins wrapped lines", () => {
    assert.deepEqual(parseChangelog(sample), [
      {
        version: "1.2.0",
        date: "2026-12-01",
        summary: "A summary that wraps onto two lines.",
        sections: [{ kind: "Fixed", items: ["A fix that wraps onto a second line.", "Another fix."] }],
      },
    ]);
  });

  it("rejects an undated release and an unknown section", () => {
    assert.throws(() => parseChangelog("## [1.0.0]\n"), /no date/);
    assert.throws(() => parseChangelog("## [1.0.0] - 2026-01-01\n### Misc\n"), /unknown section/);
  });
});

describe("inline", () => {
  it("escapes HTML and renders code, bold and links", () => {
    assert.equal(
      inline("Use `<b>` with **care**, see [docs](https://example.com) & more"),
      'Use <code>&lt;b&gt;</code> with <strong>care</strong>, see <a href="https://example.com">docs</a> &amp; more',
    );
  });

  it("does not turn javascript: URLs into links", () => {
    assert.equal(inline("[x](javascript:alert(1))"), "[x](javascript:alert(1))");
  });
});
