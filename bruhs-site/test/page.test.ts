import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { describe, it } from "node:test";

import { COMMANDS } from "../public/palette.js";

const PUBLIC = new URL("../public/", import.meta.url);
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

  it("only references images that exist", () => {
    const sources = [...html.matchAll(/\s(?:src|srcset)="([^"]+)"/g)]
      .flatMap((match) => (match[1] ?? "").split(","))
      .map((candidate) => candidate.trim().split(/\s+/)[0] ?? "")
      .filter((path) => path.startsWith("/") && !path.endsWith(".js"));
    assert.ok(sources.length > 0);
    for (const path of sources) {
      assert.ok(existsSync(new URL(`.${path}`, PUBLIC)), `missing ${path}`);
    }
  });

  it("gives every surface screenshot a description", () => {
    for (const match of html.matchAll(/<img\s[^>]*src="\/shots\/[^"]+"[^>]*>/g)) {
      assert.match(match[0], /\salt="[^"]{20,}"/, `screenshot needs a real alt text: ${match[0].slice(0, 80)}`);
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
