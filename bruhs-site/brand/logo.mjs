// The bruhs.ai mark: the Bruh's sphere as a disc of yellow dots on a 6×6
// grid, halftoned so the dots shrink toward the rim like a lit ball.
// Dots on a coarse grid stay crisp down to a 16px favicon, where a dithered
// face would turn to noise.
//
//   node brand/logo.mjs
//
// One source for every copy of the mark, site and app:
// - logo.svg / bruhs.svg: transparent, for dark surfaces (site nav, app sidebars)
// - favicon.svg: on a longan-950 tile, so it holds up on light browser chrome;
//   packaging/macos/make-icon.sh builds the app icon from bruhs-web's copy.

import { writeFileSync } from "node:fs";

const N = 6;
const PAD = 9;
const CELL = (64 - PAD * 2) / N;
const FACE = "#fada5e"; // bruh-base
const TILE = "#141210"; // longan-950

const round = (n) => Math.round(n * 100) / 100;

/** @returns {string} */
function dots() {
  const r = N / 2;
  let body = "";
  for (let y = 0; y < N; y++) {
    for (let x = 0; x < N; x++) {
      const dx = (x + 0.5 - r) / r;
      const dy = (y + 0.5 - r) / r;
      const d2 = dx * dx + dy * dy;
      if (d2 > 1.02) continue;
      const cx = round(PAD + (x + 0.5) * CELL);
      const cy = round(PAD + (y + 0.5) * CELL);
      // Even halftone: dots shrink toward the rim, like a lit sphere seen head-on.
      const nz = Math.sqrt(Math.max(0, 1 - d2));
      body += `<circle cx="${cx}" cy="${cy}" r="${round(CELL * (0.2 + 0.24 * Math.min(1, 0.35 + 0.75 * nz)))}"/>`;
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
const files = [
  [new URL("logo.svg", site), svg(false)],
  [new URL("favicon.svg", site), svg(true)],
  [new URL("bruhs.svg", app), svg(false)],
  [new URL("favicon.svg", app), svg(true)],
];
for (const [url, text] of files) writeFileSync(url, text);
console.log(`wrote ${files.map(([url]) => url.pathname.split("/").slice(-2).join("/")).join(", ")}`);
