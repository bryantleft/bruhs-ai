import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { describe, it } from "node:test";

import { COMMANDS } from "../src/scripts/palette.js";

// The built site: `npm test` runs `astro build` first.
const PUBLIC = new URL("../dist/", import.meta.url);
const html = readFileSync(new URL("index.html", PUBLIC), "utf8");
const ids = new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((match) => match[1]));

describe("index.html", () => {
  it("has an element or a page for every command-bar target", () => {
    for (const command of COMMANDS) {
      const found = command.target.startsWith("#")
        ? ids.has(command.target.slice(1))
        : existsSync(new URL(`.${command.target}.html`, PUBLIC));
      assert.ok(found, `${command.id} jumps to ${command.target}, which doesn't exist`);
    }
  });

  it("gives every Copy button an element to copy from", () => {
    const targets = [...html.matchAll(/data-copy="([^"]+)"/g)].map((match) => match[1] ?? "");
    assert.ok(targets.length > 0);
    for (const target of targets) assert.ok(ids.has(target), `Copy button points at missing #${target}`);
  });

  it("only references images that exist", () => {
    const sources = [...html.matchAll(/\s(?:src|srcset|data-src|data-srcset)="([^"]+)"/g)]
      .flatMap((match) => (match[1] ?? "").split(","))
      .map((candidate) => candidate.trim().split(/\s+/)[0] ?? "")
      .filter((path) => path.startsWith("/") && !path.endsWith(".js"));
    assert.ok(sources.length > 0);
    for (const path of sources) {
      assert.ok(existsSync(new URL(`.${path}`, PUBLIC)), `missing ${path}`);
    }
  });

  it("gives every surface screenshot a description", () => {
    for (const match of html.matchAll(/<img\s[^>]*(?:src|data-src)="\/shots\/[^"]+"[^>]*>/g)) {
      assert.match(match[0], /\salt="[^"]{20,}"/, `screenshot needs a real alt text: ${match[0].slice(0, 80)}`);
    }
  });
  it("loads only the first tour image until another tab is selected", () => {
    const panels = [...html.matchAll(/<div class="panel"([^>]*)>([\s\S]*?)<\/figure>/g)];
    assert.equal(panels.length, 7);
    for (const [, attributes, content] of panels) {
      const image = content?.match(/<img\s[^>]*>/)?.[0];
      assert.ok(image, "every surface needs its screenshot");
      if (/\shidden(?:\s|$)/.test(attributes ?? "")) {
        assert.doesNotMatch(image, /\ssrc(?:set)?=/, "inactive images must not contend with fonts and the selected screenshot");
        assert.match(image, /\sdata-src="\/shots\//);
      } else {
        assert.match(image, /\ssrc="\/shots\//);
      }
    }
  });
});

describe("changelog.html", () => {
  const changelog = readFileSync(new URL("changelog.html", PUBLIC), "utf8");
  const releases = [...changelog.matchAll(/<article class="release" id="v(\d+\.\d+\.\d+)">([\s\S]*?)<\/article>/g)];

  it("lists at least one release", () => {
    assert.ok(releases.length > 0);
  });

  it("orders releases newest first", () => {
    const versions = releases.map((match) => (match[1] ?? "").split(".").map(Number));
    for (let i = 1; i < versions.length; i += 1) {
      const [a = [], b = []] = [versions[i - 1], versions[i]];
      const newer = a[0]! - b[0]! || a[1]! - b[1]! || a[2]! - b[2]!;
      assert.ok(newer > 0, `v${releases[i - 1]?.[1]} is listed before an equal or newer v${releases[i]?.[1]}`);
    }
  });

  it("links each release title to itself and to its GitHub release", () => {
    for (const [, version, body] of releases) {
      assert.match(body ?? "", new RegExp(`<h2 class="release-title"><a href="#v${version}">`), `v${version} title link`);
      assert.match(body ?? "", new RegExp(`/releases/tag/v${version}"`), `v${version} release link`);
    }
  });

  it("marks only the newest release as Current", () => {
    const current = releases.map(([, , body]) => (body ?? "").includes("badge-current"));
    assert.deepEqual(current, releases.map((_, index) => index === 0));
  });
});

describe("download.html", () => {
  const downloads = readFileSync(new URL("download.html", PUBLIC), "utf8");
  const downloadScript = readFileSync(new URL("../src/scripts/download.ts", import.meta.url), "utf8");
  const downloadIds = new Set([...downloads.matchAll(/\sid="([^"]+)"/g)].map((match) => match[1]));

  it("offers the three supported package formats with a working offline-metadata fallback", () => {
    const names = [...downloads.matchAll(/data-package="([^"]+)"/g)].map((match) => match[1]);
    assert.deepEqual(names, ["Bruhs-macOS-arm64.dmg", "Bruhs-Windows-x64-Setup.exe", "Bruhs-Linux-x64.deb"]);
    assert.equal(downloads.match(/href="https:\/\/github.com\/bryantleft\/bruhs-ai\/releases\/latest" data-package-link/g)?.length, names.length);
  });

  it("renders official OS logos and badges without middle-dot or bullet separators", () => {
    for (const icon of ["apple", "windows", "linux"]) {
      const matches = downloads.match(new RegExp(`icon-${icon}`, "g")) ?? [];
      assert.ok(matches.length >= 2, `expected icon-${icon} in hero CTA and platform card`);
    }
    assert.ok((downloads.match(/class="badge\b/g)?.length ?? 0) >= 10, "expected platform and release badges");
    assert.doesNotMatch(downloads, /[·•]/, "download.html should use badges instead of middle-dot or bullet separators");
    assert.doesNotMatch(downloadScript, /[·•]/, "download.ts should use badges instead of middle-dot or bullet separators");
  });

  it("wires the source command Copy button to an existing element", () => {
    const targets = [...downloads.matchAll(/data-copy="([^"]+)"/g)].map((match) => match[1] ?? "");
    assert.ok(targets.length > 0);
    for (const target of targets) assert.ok(downloadIds.has(target), `Copy button points at missing #${target}`);
  });
});
