// The bruhs.ai mark: a circle of yellow dots, the Bruh's sphere drawn in the
// site's dot texture. One dot in the center and three rings around it, each
// ring holding as many dots as fit its circumference evenly, so the outline
// is a true circle and still reads as a solid disc at a 16px favicon.
//
//   node brand/logo.mjs
//
// One source for every copy of the mark, site and app:
// - logo.svg / bruhs.svg / favicon.svg: transparent dots (site nav, app
//   sidebars, browser tabs on the site and the web app)
// - packaging/macos/AppIcon.svg: on a longan-950 tile, since a Mac icon needs
//   its own background; packaging/macos/make-icon.sh builds AppIcon.icns from it.

import { writeFileSync } from "node:fs";

const RINGS = 3;
// Disc radius in the 64-unit box, and each dot's radius as a share of the ring gap.
const RADIUS = 27;
const DOT = 0.42;
const FACE = "#fada5e"; // bruh-base
const TILE = "#141210"; // longan-950

const round = (n) => Math.round(n * 100) / 100;

/** @returns {string} */
function dots() {
  const gap = RADIUS / (RINGS + 0.5);
  const r = round(gap * DOT);
  let body = `<circle cx="32" cy="32" r="${r}"/>`;
  for (let ring = 1; ring <= RINGS; ring++) {
    const count = Math.round(2 * Math.PI * ring);
    // Stagger alternate rings by half a step so the dots nest.
    const phase = ring % 2 ? 0 : Math.PI / count;
    for (let i = 0; i < count; i++) {
      const a = phase + (i / count) * Math.PI * 2 - Math.PI / 2;
      body += `<circle cx="${round(32 + ring * gap * Math.cos(a))}" cy="${round(32 + ring * gap * Math.sin(a))}" r="${r}"/>`;
    }
  }
  return `<g fill="${FACE}">${body}</g>`;
}

/** @param {boolean} tile */
function svg(tile) {
  const bg = tile ? `<rect width="64" height="64" rx="14" fill="${TILE}"/>` : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">${bg}${dots()}</svg>\n`;
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
