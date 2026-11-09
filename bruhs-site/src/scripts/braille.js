// The Bruh mascot as a halftone of dots: a hex grid over the face, each dot
// sized by the light on a sphere (dot area tracks brightness) and shrunk a
// little toward the rim, so the silhouette stays round and the face reads as
// a lit ball. The eyes are holes and the mouth is one row of red. Geometry
// from bruhs.svg; `lookX` turns toward its `left` and `right` variants.
// The same algorithm lives in @bruhs/theme/dither and bruhs-models
// logic/dither.rs. No DOM here: tests run it in Node.

/** 4×4 Bayer matrix as thresholds in (0, 1); the screenshot dissolve's order. */
const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map((n) => (n + 0.5) / 16);

/**
 * Ordered-dither threshold for a grid position.
 * @param {number} col
 * @param {number} row
 */
export function threshold(col, row) {
  return BAYER[(row & 3) * 4 + (col & 3)] ?? 0.5;
}

/**
 * Stable noise in [0, 1) per grid position: the order dots appear in a reveal.
 * @param {number} col
 * @param {number} row
 */
export function noise(col, row) {
  const n = Math.sin(col * 12.9898 + row * 78.233) * 43758.5453;
  return n - Math.floor(n);
}

// Light from the upper left, mostly frontal; some ambient so the shadow side
// keeps its dots and the outline stays whole.
const LIGHT = normalize([-0.35, -0.45, 0.82]);
const AMBIENT = 0.25;
const RIM = 0.35;

// Mascot geometry in face radii (bruhs.svg: r = 300, center 300,300).
/** @param {number} x @param {number} y */
const unit = (x, y) => ({ x: (x - 300) / 300, y: (y - 300) / 300 });
const FACES = {
  original: { leftEye: unit(198, 223), rightEye: unit(401, 223), mouth: unit(307, 378) },
  left: { leftEye: unit(97, 209), rightEye: unit(300, 209), mouth: unit(191, 364) },
  right: { leftEye: unit(300, 209), rightEye: unit(503, 209), mouth: unit(409, 364) },
};
const EYE_RX = 40 / 300;
const MOUTH_RX = 120 / 300;
const FEATURE_RY = 10 / 300;

export const FACE = 1;
export const MOUTH = 2;

/**
 * @typedef {object} Face
 * @property {number} cx      Center, in canvas pixels.
 * @property {number} cy
 * @property {number} r       Radius, in canvas pixels.
 * @property {number} [lookX] -1 turns fully to the logo's `left` variant, 1 to `right`.
 * @property {number} [lookY] -1 (up) to 1 (down), a small nudge.
 */

/**
 * @typedef {object} Dot
 * @property {number} x
 * @property {number} y
 * @property {number} radius
 * @property {1 | 2} kind  FACE or MOUTH.
 * @property {number} col  Grid position, for the reveal order.
 * @property {number} row
 */

/**
 * Grid spacing for a face radius: about 15 dots per radius on a large face,
 * down to 9 on a small one (fewer turns the outline into a hexagon).
 * @param {number} r
 */
export function pitchFor(r) {
  return r / Math.max(9, Math.min(15, r / 10));
}

/**
 * Brightness of the sphere at a point in face radii, or null outside it.
 * @param {number} dx
 * @param {number} dy
 */
export function shade(dx, dy) {
  const d2 = dx * dx + dy * dy;
  if (d2 > 1) return null;
  const nz = Math.sqrt(1 - d2);
  const lambert = Math.max(0, LIGHT[0] * dx + LIGHT[1] * dy + LIGHT[2] * nz);
  return (AMBIENT + (1 - AMBIENT) * lambert) * (1 - RIM + RIM * Math.sqrt(nz));
}

/**
 * Every dot of a Bruh in a `width` × `height` box. The hex grid is anchored on
 * the face's center, so the face is symmetric wherever it sits.
 * @param {number} width
 * @param {number} height
 * @param {Face} face
 * @returns {Dot[]}
 */
export function bruhDots(width, height, face) {
  const { cx, cy, r } = face;
  const pitch = pitchFor(r);
  const rowH = (pitch * Math.sqrt(3)) / 2;
  const look = Math.max(-1, Math.min(1, face.lookX ?? 0));
  const nudge = Math.max(-1, Math.min(1, face.lookY ?? 0)) * 0.05;
  const turned = look < 0 ? FACES.left : FACES.right;
  const t = Math.abs(look);
  /** @param {{ x: number, y: number }} a @param {{ x: number, y: number }} b */
  const toward = (a, b) => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t + nudge });
  const eyes = [toward(FACES.original.leftEye, turned.leftEye), toward(FACES.original.rightEye, turned.rightEye)];
  const mouth = toward(FACES.original.mouth, turned.mouth);
  const eyeRy = Math.max(FEATURE_RY, (pitch * 0.9) / r);
  // The mouth is exactly one row: the one nearest its line.
  const mouthRow = Math.round((mouth.y * r) / rowH);

  /** @type {Dot[]} */
  const dots = [];
  const rows = Math.ceil(r / rowH) + 1;
  const cols = Math.ceil(r / pitch) + 1;
  for (let row = -rows; row <= rows; row++) {
    const y = cy + row * rowH;
    if (y < -pitch || y > height + pitch) continue;
    const offset = row & 1 ? pitch / 2 : 0;
    for (let col = -cols; col <= cols; col++) {
      const x = cx + col * pitch + offset;
      if (x < -pitch || x > width + pitch) continue;
      const dx = (x - cx) / r;
      const dy = (y - cy) / r;
      let value = shade(dx, dy);
      if (value === null) continue;
      if (eyes.some((e) => ((dx - e.x) / EYE_RX) ** 2 + ((dy - e.y) / eyeRy) ** 2 <= 1)) continue;
      const isMouth = row === mouthRow && Math.abs((dx - mouth.x) / MOUTH_RX) <= 1;
      if (isMouth) value = Math.max(value, 0.75);
      const radius = (pitch / 2) * 0.92 * Math.sqrt(value);
      if (radius < pitch * 0.07) continue;
      dots.push({ x, y, radius, kind: isMouth ? MOUTH : FACE, col, row });
    }
  }
  return dots;
}

/**
 * Draws a Bruh, one path per color. `reveal` below 1 keeps that share of the
 * dots, in `noise` order, for an entrance; leave it at 1 under reduced motion.
 * @param {CanvasRenderingContext2D} ctx
 * @param {number} width
 * @param {number} height
 * @param {Face} face
 * @param {{ reveal?: number, colors: { face: string, mouth: string } }} style
 */
export function drawBruh(ctx, width, height, face, style) {
  const reveal = style.reveal ?? 1;
  const paths = { [FACE]: new Path2D(), [MOUTH]: new Path2D() };
  for (const dot of bruhDots(width, height, face)) {
    if (reveal < 1 && noise(dot.col, dot.row) > reveal) continue;
    const path = paths[dot.kind];
    path.moveTo(dot.x + dot.radius, dot.y);
    path.arc(dot.x, dot.y, dot.radius, 0, Math.PI * 2);
  }
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = style.colors.face;
  ctx.fill(paths[FACE]);
  ctx.fillStyle = style.colors.mouth;
  ctx.fill(paths[MOUTH]);
}

/** @param {[number, number, number]} v @returns {[number, number, number]} */
function normalize([x, y, z]) {
  const length = Math.hypot(x, y, z);
  return [x / length, y / length, z / length];
}
