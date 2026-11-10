// The Bruh mascot as a halftone of dots: a hex grid over the face, each dot
// sized by the light on a sphere (dot area tracks brightness) and shrunk a
// little toward the rim, so the silhouette stays round and the face reads as
// a lit ball. The eyes are one-row slits and the mouth is one row of red.
// Looking turns the whole head in 3D: the features ride the sphere's surface,
// curving and foreshortening toward the rim under a fixed light, and the ball
// leans a little the way it looks. Geometry from bruhs.svg. The same algorithm
// lives in @bruhs/theme/dither and bruhs-models logic/dither.rs (static there).
// No DOM here: tests run it in Node.

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

// Mascot geometry in face radii (bruhs.svg: r = 300, center 300,300). The logo
// has the eyes 102 and 101 units out; they're mirrored here so the symmetric
// hex grid cuts identical slits.
/** @param {number} x @param {number} y */
const unit = (x, y) => ({ x: (x - 300) / 300, y: (y - 300) / 300 });
const LEFT_EYE = unit(198.5, 223);
const RIGHT_EYE = unit(401.5, 223);
const MOUTH_AT = unit(307, 378);
const EYE_RX = 40 / 300;
const MOUTH_RX = 120 / 300;

// A full look (lookX or lookY of ±1) turns the head this far, in radians,
// and shifts the ball this share of its radius.
const MAX_YAW = 0.75;
const MAX_PITCH = 0.45;
const LEAN = 0.04;

export const FACE = 1;
export const MOUTH = 2;

/**
 * @typedef {object} Face
 * @property {number} cx      Center, in canvas pixels.
 * @property {number} cy
 * @property {number} r       Radius, in canvas pixels.
 * @property {number} [lookX] -1 turns the head fully left, 1 fully right.
 * @property {number} [lookY] -1 tilts it up, 1 down.
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
 * the ball's center, so a face looking straight ahead is exactly symmetric.
 * @param {number} width
 * @param {number} height
 * @param {Face} face
 * @returns {Dot[]}
 */
export function bruhDots(width, height, face) {
  const { r } = face;
  const lookX = Math.max(-1, Math.min(1, face.lookX ?? 0));
  const lookY = Math.max(-1, Math.min(1, face.lookY ?? 0));
  const cx = face.cx + lookX * LEAN * r;
  const cy = face.cy + lookY * LEAN * r;
  const pitch = pitchFor(r);
  const rowH = (pitch * Math.sqrt(3)) / 2;
  const head = rotation(lookX * MAX_YAW, lookY * MAX_PITCH);

  // Where each feature lands once the head turns. Eyes and mouth are exactly
  // one row each, the row nearest where they land: an oval hole cut across
  // staggered rows comes out ▽ on one eye and △ on the other.
  /** @param {{ x: number, y: number }} at */
  const land = (at) => {
    const seen = head.toScreen(lift(at));
    return { at, row: Math.round((seen.y * r) / rowH), visible: seen.z > 0.15 };
  };
  const leftEye = land(LEFT_EYE);
  const rightEye = land(RIGHT_EYE);
  const mouth = land(MOUTH_AT);
  /**
   * @param {{ at: { x: number, y: number }, row: number, visible: boolean }} feature
   * @param {number} row
   * @param {{ x: number, y: number }} local
   * @param {number} rx
   */
  const on = (feature, row, local, rx) =>
    feature.visible && row === feature.row && Math.abs(local.x - feature.at.x) <= rx && Math.abs(local.y - feature.at.y) < 0.2;

  /** @type {Dot[]} */
  const dots = [];
  const rows = Math.ceil(r / rowH) + 1;
  const cols = Math.ceil(r / pitch) + 1;
  for (let row = -rows; row <= rows; row++) {
    const y = cy + row * rowH;
    if (y < -pitch || y > height + pitch) continue;
    for (let col = -cols; col <= cols; col++) {
      // Offsets in whole half-steps, so a dot and its mirror twin get exactly
      // negated positions and a front-facing face stays symmetric.
      const halfSteps = 2 * col + (row & 1);
      const x = cx + (halfSteps * pitch) / 2;
      if (x < -pitch || x > width + pitch) continue;
      const dx = (halfSteps * pitch) / (2 * r);
      const dy = (row * rowH) / r;
      let value = shade(dx, dy);
      if (value === null) continue;
      // The point of the head's own (unturned) surface now under this dot.
      const local = head.toLocal(dx, dy, Math.sqrt(Math.max(0, 1 - dx * dx - dy * dy)));
      if (on(leftEye, row, local, EYE_RX) || on(rightEye, row, local, EYE_RX)) continue;
      const isMouth = on(mouth, row, local, MOUTH_RX);
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

/**
 * A point of the front-facing face, in face radii, lifted onto the sphere.
 * @param {{ x: number, y: number }} p
 */
function lift(p) {
  return { x: p.x, y: p.y, z: Math.sqrt(Math.max(0, 1 - p.x * p.x - p.y * p.y)) };
}

/**
 * The head's turn: a tilt (pitch, positive looks down) then a turn (yaw,
 * positive looks right), on screen axes with y down and z toward the viewer.
 * @param {number} yaw
 * @param {number} pitch
 */
function rotation(yaw, pitch) {
  const cosY = Math.cos(yaw);
  const sinY = Math.sin(yaw);
  const cosP = Math.cos(pitch);
  const sinP = Math.sin(pitch);
  return {
    /** @param {{ x: number, y: number, z: number }} p */
    toScreen(p) {
      const y = p.y * cosP + p.z * sinP;
      const z = -p.y * sinP + p.z * cosP;
      return { x: p.x * cosY + z * sinY, y, z: -p.x * sinY + z * cosY };
    },
    /** @param {number} x @param {number} y @param {number} z */
    toLocal(x, y, z) {
      const lx = x * cosY - z * sinY;
      const lz = x * sinY + z * cosY;
      return { x: lx, y: y * cosP - lz * sinP, z: y * sinP + lz * cosP };
    },
  };
}

/** @param {[number, number, number]} v @returns {[number, number, number]} */
function normalize([x, y, z]) {
  const length = Math.hypot(x, y, z);
  return [x / length, y / length, z / length];
}
