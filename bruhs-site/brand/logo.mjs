// The bruhs.ai mark: the Bruh itself (bruhs.svg's face, eyes and mouth
// exactly) as a dotted sphere. The ball is solid yellow to 78% of its radius,
// then breaks into three rings of dots that shrink outward and toward the
// shadow side, lit like the site's dot Bruh (src/scripts/braille.js `shade`).
// The rings are concentric, so the outline stays a true circle, and the face
// keeps the mascot's crisp features, so it still reads as the Bruh at a 16px
// favicon.
//
//   node brand/logo.mjs
//
// One source for every copy of the mark, site and app:
// - logo.svg / bruhs.svg / favicon.svg: transparent (site nav, app sidebars,
//   browser tabs on the site and the web app)
// - packaging/macos/AppIcon.svg: on a longan-950 tile, since a Mac icon needs
//   its own background; packaging/macos/make-icon.sh builds AppIcon.icns from it.

import { writeFileSync } from "node:fs";
import { shade } from "../src/scripts/braille.js";

const FACE = "#fada5e"; // bruh-base
const MOUTH = "#fc4141"; // bruh-mouth
const EYE = "#2e2e29"; // bruh-eye in bruhs.svg
const TILE = "#141210"; // longan-950

// Share of the radius that stays solid, and the rings of dots outside it.
const CORE = 0.78;
const RINGS = 3;

const round = (n) => Math.round(n * 100) / 100;

/** The Bruh in bruhs.svg's 600-unit box (center 300,300, radius 300). */
function bruh() {
  const gap = (1 - CORE) / RINGS;
  let dots = `<circle cx="300" cy="300" r="${round(CORE * 300)}"/>`;
  for (let ring = 1; ring <= RINGS; ring++) {
    const radius = CORE + (ring - 0.5) * gap;
    // Dots about a ring gap apart, each ring staggered half a step.
    const count = Math.round((2 * Math.PI * radius) / (gap * 1.1));
    const phase = ring % 2 ? 0 : Math.PI / count;
    for (let i = 0; i < count; i++) {
      const a = phase + (i / count) * 2 * Math.PI;
      const dx = radius * Math.cos(a);
      const dy = radius * Math.sin(a);
      const light = shade(dx, dy) ?? 0;
      // Smaller per ring outward, and smaller where the light falls off.
      const size = (gap / 2) * (1 - (ring - 1) / (RINGS + 0.6)) * Math.min(1, Math.sqrt(light) * 1.5);
      if (size < gap * 0.08) continue;
      dots += `<circle cx="${round(300 + dx * 300)}" cy="${round(300 + dy * 300)}" r="${round(size * 300)}"/>`;
    }
  }
  const features =
    `<ellipse cx="198" cy="223" rx="40" ry="10" fill="${EYE}"/>` +
    `<ellipse cx="401" cy="223" rx="40" ry="10" fill="${EYE}"/>` +
    `<ellipse cx="307" cy="378" rx="120" ry="10" fill="${MOUTH}"/>`;
  return `<g fill="${FACE}">${dots}</g>${features}`;
}

/** @param {boolean} tile */
function svg(tile) {
  // On the tile the Bruh spans 84% of it, the share the earlier mark had.
  const body = tile
    ? `<rect width="600" height="600" rx="131.25" fill="${TILE}"/><g transform="translate(48 48) scale(.84)">${bruh()}</g>`
    : bruh();
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 600">${body}</svg>\n`;
}

const site = new URL("../public/", import.meta.url);
const app = new URL("../../bruhs-web/", import.meta.url);
const mac = new URL("../../packaging/macos/", import.meta.url);
const files = [
  [new URL("logo.svg", site), svg(false)],
  [new URL("favicon.svg", site), svg(false)],
  [new URL("bruhs.svg", app), svg(false)],
  [new URL("favicon.svg", app), svg(false)],
  [new URL("AppIcon.svg", mac), svg(true)],
];
for (const [url, text] of files) writeFileSync(url, text);
console.log(`wrote ${files.map(([url]) => url.pathname.split("/").slice(-2).join("/")).join(", ")}`);
