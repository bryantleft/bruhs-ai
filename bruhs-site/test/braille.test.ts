import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { FACE, MOUTH, bruhDots, pitchFor, shade, threshold } from "../src/scripts/braille.js";

const face = { cx: 200, cy: 200, r: 150 };
const dots = bruhDots(400, 400, face);

describe("threshold", () => {
  it("uses every Bayer level once per 4×4 tile", () => {
    const levels = new Set<number>();
    for (let row = 0; row < 4; row++) for (let col = 0; col < 4; col++) levels.add(threshold(col, row));
    assert.equal(levels.size, 16);
  });
});

describe("shade", () => {
  it("is brightest toward the upper left and nothing outside the sphere", () => {
    assert.ok((shade(-0.3, -0.4) ?? 0) > (shade(0.5, 0.5) ?? 1));
    assert.equal(shade(0.8, 0.8), null);
  });

  it("keeps the shadow side visible so the outline stays round", () => {
    assert.ok((shade(0.9, 0.3) ?? 0) > 0.1);
  });
});

describe("bruhDots", () => {
  it("scales the grid with the face: finer when large, never below 9 dots per radius", () => {
    assert.equal(pitchFor(150), 10);
    assert.equal(pitchFor(40), 40 / 9);
  });

  it("stays inside the face circle and is symmetric left to right", () => {
    for (const dot of dots) assert.ok(Math.hypot(dot.x - face.cx, dot.y - face.cy) <= face.r + 0.001);
    const left = dots.filter((d) => d.x < face.cx - 1).length;
    const right = dots.filter((d) => d.x > face.cx + 1).length;
    assert.ok(Math.abs(left - right) <= 2, `left ${left} vs right ${right}`);
  });

  it("makes the lit side's dots bigger than the shadow side's", () => {
    const near = (x: number, y: number) =>
      dots.reduce((best, d) => (Math.hypot(d.x - x, d.y - y) < Math.hypot(best.x - x, best.y - y) ? d : best));
    assert.ok(near(140, 120).radius > near(280, 300).radius);
  });

  it("leaves holes for the eyes and draws the mouth as one red row", () => {
    const at = (x: number, y: number) => dots.some((d) => Math.hypot(d.x - x, d.y - y) < 4);
    assert.equal(at(200 - 0.34 * 150, 200 - 0.257 * 150), false);
    assert.equal(at(200 + 0.337 * 150, 200 - 0.257 * 150), false);
    const mouth = dots.filter((d) => d.kind === MOUTH);
    assert.ok(mouth.length >= 8);
    assert.equal(new Set(mouth.map((d) => d.row)).size, 1);
    assert.ok(dots.filter((d) => d.kind === FACE).length > 500);
  });

  it("turns the features toward the pointer", () => {
    const mouthX = (lookX: number) => {
      const row = bruhDots(400, 400, { ...face, lookX }).filter((d) => d.kind === MOUTH);
      return row.reduce((sum, d) => sum + d.x, 0) / row.length;
    };
    assert.ok(mouthX(-1) < mouthX(0) - 20);
    assert.ok(mouthX(1) > mouthX(0) + 20);
  });
});
