// The bruh mascot as a field of braille dots: 2×4 dot cells, shaded like a
// lit sphere and ordered-dithered (4×4 Bayer), so the face is solid where the
// light hits and breaks into scattered dots toward the edge. Geometry follows
// bruhs.svg. No DOM here: drawing takes any 2D context, so tests run in Node.

/** 4×4 Bayer matrix as thresholds in (0, 1). */
const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map((n) => (n + 0.5) / 16);

/**
 * Ordered-dither threshold for a dot at grid position (col, row).
 * @param {number} col
 * @param {number} row
 */
export function threshold(col, row) {
  return BAYER[(row & 3) * 4 + (col & 3)] ?? 0.5;
}

/**
 * Cheap stable noise in [0, 1) for a grid position, used for the reveal order.
 * @param {number} col
 * @param {number} row
 */
export function noise(col, row) {
  const n = Math.sin(col * 12.9898 + row * 78.233) * 43758.5453;
  return n - Math.floor(n);
}

// Mascot geometry in face radii, from bruhs.svg (r = 300 there).
const EYE_X = 0.336;
const EYE_Y = -0.257;
const EYE_RX = 0.133;
const MOUTH_X = 0.023;
const MOUTH_Y = 0.26;
const MOUTH_RX = 0.4;
const FEATURE_RY = 0.033;
// Light from the upper left, mostly frontal.
const LIGHT_X = -0.36;
const LIGHT_Y = -0.42;
const LIGHT_Z = 0.83;

export const FACE = 1;
export const MOUTH = 2;
export const HALO = 3;

/**
 * @typedef {object} Face
 * @property {number} cx      Center, in canvas pixels.
 * @property {number} cy
 * @property {number} r       Radius, in canvas pixels.
 * @property {number} lookX   Where the eyes point, -1 (left) to 1 (right).
 * @property {number} lookY   -1 (up) to 1 (down).
 * @property {number} minRy   Thinnest a feature may be, in face radii, so the eyes survive the dot grid.
 */

/**
 * What sits under a point: which layer, and how bright (0 to 1). Dots whose
 * brightness beats their dither threshold are drawn.
 * @param {number} x
 * @param {number} y
 * @param {Face} face
 * @returns {{ kind: 0 | 1 | 2 | 3, value: number }}
 */
export function sample(x, y, face) {
  const dx = (x - face.cx) / face.r;
  const dy = (y - face.cy) / face.r;
  const d2 = dx * dx + dy * dy;
  const ry = Math.max(FEATURE_RY, face.minRy);

  if (d2 > 1) {
    // Fades to nothing a fifth of a radius out, inside any canvas edge.
    const fade = Math.max(0, 1 - (Math.sqrt(d2) - 1) / 0.2);
    return { kind: HALO, value: 0.24 * fade * fade };
  }

  const mx = (dx - MOUTH_X) / MOUTH_RX;
  const my = (dy - MOUTH_Y) / ry;
  if (mx * mx + my * my <= 1) return { kind: MOUTH, value: 1 };

  const ey = (dy - EYE_Y - face.lookY * 0.05) / ry;
  for (const side of [-1, 1]) {
    const ex = (dx - side * EYE_X - face.lookX * 0.07) / EYE_RX;
    if (ex * ex + ey * ey <= 1) return { kind: 0, value: 0 };
  }

  const nz = Math.sqrt(1 - d2);
  const lambert = Math.max(0, LIGHT_X * dx + LIGHT_Y * dy + LIGHT_Z * nz);
  return { kind: FACE, value: Math.min(1, 0.1 + lambert) };
}

/**
 * @typedef {object} Style
 * @property {number} pitch    Distance between dots inside a cell, in canvas pixels.
 * @property {number} gap      Extra space between cells.
 * @property {number} dot      Dot radius.
 * @property {number} reveal   0 to 1: how much of the field has resolved (1 = all).
 * @property {{ face: string, mouth: string, halo: string }} colors
 */

/**
 * Draws the whole field. One path per color keeps it to three fills.
 * @param {CanvasRenderingContext2D} ctx
 * @param {number} width
 * @param {number} height
 * @param {Face} face
 * @param {Style} style
 */
export function drawBruh(ctx, width, height, face, style) {
  const { pitch, gap, dot, reveal, colors } = style;
  const cellW = pitch * 2 + gap;
  const cellH = pitch * 4 + gap;
  const cols = Math.ceil(width / cellW);
  const rows = Math.ceil(height / cellH);
  // Center the grid so the field doesn't hug one edge.
  const ox = (width - cols * cellW + gap + pitch) / 2;
  const oy = (height - rows * cellH + gap + pitch) / 2;

  const paths = { [FACE]: new Path2D(), [MOUTH]: new Path2D(), [HALO]: new Path2D() };
  for (let cr = 0; cr < rows; cr++) {
    for (let cc = 0; cc < cols; cc++) {
      for (let dr = 0; dr < 4; dr++) {
        for (let dc = 0; dc < 2; dc++) {
          const col = cc * 2 + dc;
          const row = cr * 4 + dr;
          if (reveal < 1 && noise(col, row) > reveal) continue;
          const x = ox + cc * cellW + dc * pitch;
          const y = oy + cr * cellH + dr * pitch;
          const { kind, value } = sample(x, y, face);
          if (kind === 0 || value <= threshold(col, row)) continue;
          const path = paths[kind];
          path.moveTo(x + dot, y);
          path.arc(x, y, dot, 0, Math.PI * 2);
        }
      }
    }
  }

  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = colors.halo;
  ctx.fill(paths[HALO]);
  ctx.fillStyle = colors.face;
  ctx.fill(paths[FACE]);
  ctx.fillStyle = colors.mouth;
  ctx.fill(paths[MOUTH]);
}
