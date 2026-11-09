// The bruhs.ai mark: the Bruh as a machine. A disc of dots on a 6×6 grid
// (the mascot's sphere, halftoned) with the eye row lit as a blue visor.
// Dots on a coarse grid stay crisp down to a 16px favicon, where a dithered
// face would turn to noise.
//
//   node brand/logo.mjs   # writes public/logo.svg and public/favicon.svg
//
// logo.svg is transparent, for dark surfaces such as the nav; favicon.svg
// sits on a longan-950 tile so it holds up on light browser chrome too.

import { writeFileSync } from "node:fs";

const N = 6;
const VISOR_ROW = 2;
const PAD = 9;
const CELL = (64 - PAD * 2) / N;
const FACE = "#fada5e"; // bruh-base
const VISOR = "#9ec1ff"; // blueberry-300
const TILE = "#141210"; // longan-950

const round = (n) => Math.round(n * 100) / 100;

/** @returns {string} */
function dots() {
  const r = N / 2;
  let glow = "";
  let body = "";
  for (let y = 0; y < N; y++) {
    for (let x = 0; x < N; x++) {
      const dx = (x + 0.5 - r) / r;
      const dy = (y + 0.5 - r) / r;
      const d2 = dx * dx + dy * dy;
      if (d2 > 1.02) continue;
      const cx = round(PAD + (x + 0.5) * CELL);
      const cy = round(PAD + (y + 0.5) * CELL);
      if (y === VISOR_ROW) {
        glow += `<circle cx="${cx}" cy="${cy}" r="${round(CELL * 0.7)}"/>`;
        body += `<circle cx="${cx}" cy="${cy}" r="${round(CELL * 0.46)}" fill="${VISOR}"/>`;
      } else {
        // Even halftone: dots shrink toward the rim, like a lit sphere seen head-on.
        const nz = Math.sqrt(Math.max(0, 1 - d2));
        body += `<circle cx="${cx}" cy="${cy}" r="${round(CELL * (0.2 + 0.24 * Math.min(1, 0.35 + 0.75 * nz)))}" fill="${FACE}"/>`;
      }
    }
  }
  return `<g fill="${VISOR}" opacity=".22">${glow}</g>${body}`;
}

/** @param {boolean} tile */
function svg(tile) {
  const bg = tile ? `<rect width="64" height="64" rx="14" fill="${TILE}"/>` : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">${bg}${dots()}</svg>\n`;
}

const here = new URL("../public/", import.meta.url);
writeFileSync(new URL("logo.svg", here), svg(false));
writeFileSync(new URL("favicon.svg", here), svg(true));
console.log("wrote public/logo.svg and public/favicon.svg");
