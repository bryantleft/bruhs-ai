// The bruhs.ai mark: the site's dot Bruh, 1:1. Every dot comes from the same
// bruhDots as the hero (src/scripts/braille.js), looking straight ahead, and
// the mouth takes drawBruh's five yellow-to-red steps.
//
//   node brand/logo.mjs
//
// bruhDots picks its grid from the Bruh's radius in pixels: 9 dots per radius
// up to r 90, 15 from r 150. An SVG scales, so each file is generated at the
// radius it is shown at, which makes it the same dots the site would draw
// there:
// - small (r 80, 9 per radius): logo.svg / bruhs.svg / favicon.svg, shown at
//   16 to 52px (site nav, app sidebars, browser tabs), and AppIcon-small.svg
//   for Mac icon sizes up to 256px, where the art's radius stays under 90px.
// - large (r 200, 15 per radius): AppIcon.svg for the 512 and 1024px sizes.
// packaging/macos/make-icon.sh builds AppIcon.icns from the two AppIcon SVGs,
// whose longan-950 tile a Mac icon needs as its own background.

import { writeFileSync } from "node:fs";
import { bruhDots, EYE_RX, gridFor, LEFT_EYE, pitchFor, RIGHT_EYE } from "../src/scripts/braille.js";

// The site's BRUH_COLORS (src/scripts/site.js).
const FACE = "#fada5e";
const MOUTH = "#fc4141";
const EYE = "#302720"; // Bruhs light rambutan-200
const TILE = "#141210"; // longan-950
const SMALL = 80;
const LARGE = 200;

const round = (n) => Math.round(n * 100) / 100;

/**
 * drawBruh's mouth blend: `#rrggbb` between `from` and `to`, `t` of the way.
 * @param {string} from @param {string} to @param {number} t
 */
function mixHex(from, to, t) {
  if (t <= 0) return from;
  if (t >= 1) return to;
  const a = Number.parseInt(from.slice(1), 16);
  const b = Number.parseInt(to.slice(1), 16);
  /** @param {number} shift */
  const channel = (shift) => Math.round(((a >> shift) & 255) + (((b >> shift) & 255) - ((a >> shift) & 255)) * t);
  return `#${((channel(16) << 16) | (channel(8) << 8) | channel(0)).toString(16).padStart(6, "0")}`;
}

/**
 * The Bruh at radius `r`, centered in a `box`-unit square: one group per
 * mouth step, like drawBruh's one path per color.
 * @param {number} r @param {number} box
 */
function bruh(r, box) {
  const steps = ["", "", "", "", ""];
  for (const dot of bruhDots(box, box, { cx: box / 2, cy: box / 2, r })) {
    const step = Math.round(dot.mix * 4);
    steps[step] += `<circle cx="${round(dot.x)}" cy="${round(dot.y)}" r="${round(dot.radius)}"/>`;
  }
  return steps.map((dots, i) => (dots ? `<g fill="${mixHex(FACE, MOUTH, i / 4)}">${dots}</g>` : "")).join("");
}

/**
 * Give the slits their own ink under the halftone dots on transparent marks.
 * @param {number} r
 * @param {number} box
 */
function eyes(r, box) {
  const center = box / 2;
  const halfWidth = EYE_RX * r;
  const height = gridFor(r).rowH;
  return [LEFT_EYE, RIGHT_EYE].map((eye) => {
    const x = round(center + eye.x * r - halfWidth);
    const y = round(center + eye.y * r - height / 2);
    return `<rect x="${x}" y="${y}" width="${round(halfWidth * 2)}" height="${round(height)}" rx="${round(height / 2)}" fill="${EYE}"/>`;
  }).join("");
}

/**
 * Transparent: the Bruh fills the box like bruhs.svg, with half a dot pitch
 * spare on each side so the rim's dots stay whole. On the tile it spans 84%
 * of it, the share the earlier mark had.
 * @param {number} r @param {boolean} tile
 */
function svg(r, tile) {
  const box = round(tile ? (2 * r) / 0.84 : 2 * r + pitchFor(r));
  const bg = tile ? `<rect width="${box}" height="${box}" rx="${round((box * 14) / 64)}" fill="${TILE}"/>` : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${box} ${box}">${bg}${tile ? "" : eyes(r, box)}${bruh(r, box)}</svg>\n`;
}

const site = new URL("../public/", import.meta.url);
const app = new URL("../../bruhs-web/", import.meta.url);
const mac = new URL("../../packaging/macos/", import.meta.url);
const files = [
  [new URL("logo.svg", site), svg(SMALL, false)],
  [new URL("favicon.svg", site), svg(SMALL, false)],
  [new URL("bruhs.svg", app), svg(SMALL, false)],
  [new URL("favicon.svg", app), svg(SMALL, false)],
  [new URL("AppIcon-small.svg", mac), svg(SMALL, true)],
  [new URL("AppIcon.svg", mac), svg(LARGE, true)],
];
for (const [url, text] of files) writeFileSync(url, text);
console.log(`wrote ${files.map(([url]) => url.pathname.split("/").slice(-2).join("/")).join(", ")}`);
