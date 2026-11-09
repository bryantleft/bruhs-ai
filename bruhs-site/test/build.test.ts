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
