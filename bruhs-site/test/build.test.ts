import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { describe, it } from "node:test";

// vercel.json's CSP is `script-src 'self'; style-src 'self'`: an inline
// script, <style> block or style="" attribute is blocked in production while
// working fine under `astro dev`. Astro inlines small scripts and styles by
// default (see astro.config.mjs), so check what the build actually emitted.
const DIST = new URL("../dist/", import.meta.url);
const pages = readdirSync(DIST).filter((name) => name.endsWith(".html"));

describe("built pages", () => {
  it("include the home page, changelog and privacy page", () => {
    assert.deepEqual([...pages].sort(), ["changelog.html", "index.html", "privacy.html"]);
  });

  for (const page of pages) {
    const html = readFileSync(new URL(page, DIST), "utf8");

    it(`${page} has no inline script, style block or style attribute`, () => {
      for (const tag of html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) {
        assert.match(tag[0], /\ssrc="\/[^"]+"/, `inline script in ${page}: ${tag[0].slice(0, 80)}`);
        assert.equal((tag[1] ?? "").trim(), "", `script with a body in ${page}`);
      }
      assert.doesNotMatch(html, /<style[\s>]/, `<style> block in ${page}`);
      assert.doesNotMatch(html, /\sstyle="/, `style="" attribute in ${page}`);
    });

    it(`${page} loads its stylesheet from the site`, () => {
      assert.match(html, /<link rel="stylesheet" href="\/_astro\/[^"]+\.css"/);
    });
  }
});

// Pages prerender on hover so a link opens without waiting on the network.
// The CSP blocks inline <script type="speculationrules">, so the rules come
// from a file the Speculation-Rules header names.
describe("speculation rules", () => {
  type Vercel = { headers: { source: string; headers: { key: string; value: string }[] }[] };
  const vercel: Vercel = JSON.parse(readFileSync(new URL("../vercel.json", import.meta.url), "utf8"));
  const header = (source: string, key: string) =>
    vercel.headers.find((rule) => rule.source === source)?.headers.find((h) => h.key === key)?.value;

  it("are named by a header on every page and served as speculation rules", () => {
    const value = header("/(.*)", "Speculation-Rules");
    assert.ok(value, "no Speculation-Rules header");
    const path = value.replace(/^"|"$/g, "");
    assert.equal(header(path, "Content-Type"), "application/speculationrules+json");
    const rules = JSON.parse(readFileSync(new URL(`.${path}`, DIST), "utf8"));
    assert.ok(rules.prerender?.length, "no prerender rule");
    for (const rule of rules.prerender) assert.equal(rule.eagerness, "moderate", "prerender on hover, not on page load");
  });
});
