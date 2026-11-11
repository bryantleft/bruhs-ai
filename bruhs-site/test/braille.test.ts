import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { FACE, MOUTH, bruhDots, gridFor, pitchFor, shade, threshold } from "../src/scripts/braille.js";

// bruhs.svg, in face radii (r = 300, center 300,300).
const EYE_Y = -77 / 300;
const MOUTH_Y = 78 / 300;

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

  it("lands the eye and mouth lines exactly where bruhs.svg puts them, at every size", () => {
    for (const r of [20, 40, 96 * 0.42, 75, 150, 208, 300]) {
      const size = Math.ceil(r * 2.5);
      const c = size / 2;
      const grid = gridFor(r);
      assert.ok(grid.pitch <= pitchFor(r) + 1e-9, `r ${r}: the fitted grid is never coarser`);
      const all = bruhDots(size, size, { cx: c, cy: c, r });
      const rowYs = new Set(all.map((d) => d.y));
      const onRow = (y: number) => [...rowYs].some((rowY) => Math.abs(rowY - y) < 1e-9);
      assert.ok(onRow(c + EYE_Y * r), `r ${r}: a row on the eye line`);
      assert.ok(onRow(c + MOUTH_Y * r), `r ${r}: a row on the mouth line`);
      for (const d of all.filter((dot) => dot.kind === MOUTH)) assert.ok(Math.abs(d.y - (c + MOUTH_Y * r)) < 1e-9, `r ${r}: red dot off the mouth line`);
    }
  });

  it("cuts the eyes as one-row slits that mirror each other", () => {
    const { pitch, rowH } = gridFor(150);
    const eyeY = 200 + EYE_Y * 150;
    const inSlit = (d: { x: number; y: number }, side: number) =>
      Math.abs(d.y - eyeY) < 1e-9 && Math.abs(d.x - (200 + side * 0.3383 * 150)) <= (40 / 300) * 150;
    assert.equal(dots.filter((d) => inSlit(d, -1) || inSlit(d, 1)).length, 0);
    // The rows above and below keep their dots at the eye: a slit, not a hole.
    for (const y of [eyeY - rowH, eyeY + rowH]) {
      assert.ok(dots.some((d) => Math.abs(d.y - y) < 1e-6 && Math.abs(d.x - (200 - 0.3383 * 150)) < pitch));
    }
    // Every face dot has a mirror twin, so both eyes are the same shape.
    const key = (x: number, y: number) => `${Math.round(x * 10)},${Math.round(y * 10)}`;
    const face_ = new Set(dots.filter((d) => d.kind === FACE).map((d) => key(d.x, d.y)));
    const unmatched = dots.filter((d) => d.kind === FACE && Math.abs(d.y - (200 + MOUTH_Y * 150)) > 6 && !face_.has(key(400 - d.x, d.y)));
    assert.equal(unmatched.length, 0);
  });

  it("draws the mouth as one red row", () => {
    const mouth = dots.filter((d) => d.kind === MOUTH);
    assert.ok(mouth.length >= 8);
    assert.equal(new Set(mouth.map((d) => d.row)).size, 1);
    assert.ok(dots.filter((d) => d.kind === FACE).length > 500);
  });

  it("turns the whole head: features ride the sphere and the ball leans", () => {
    const mouthX = (lookX: number) => {
      const row = bruhDots(400, 400, { ...face, lookX }).filter((d) => d.kind === MOUTH);
      return row.reduce((sum, d) => sum + d.x, 0) / row.length;
    };
    assert.ok(mouthX(-1) < mouthX(0) - 60);
    assert.ok(mouthX(1) > mouthX(0) + 60);

    // Turned right, both eye slits stay in one row as two gaps, and the right
    // one, now nearer the rim, is foreshortened: narrower than the left.
    const turned = bruhDots(400, 400, { ...face, lookX: 0.8 });
    const byRow = new Map<number, number[]>();
    for (const d of turned) byRow.set(d.row, [...(byRow.get(d.row) ?? []), d.x]);
    const gapsIn = (xs: number[]) => {
      const sorted = [...xs].sort((a, b) => a - b);
      return sorted.slice(1).map((x, i) => x - (sorted[i] ?? x)).filter((gap) => gap > 1.5 * pitchFor(150));
    };
    const eyeRow = [...byRow.values()].map(gapsIn).find((gaps) => gaps.length === 2);
    assert.ok(eyeRow, "a row with both eye slits");
    assert.ok((eyeRow[1] ?? 0) < (eyeRow[0] ?? 0), `right slit ${eyeRow[1]} vs left ${eyeRow[0]}`);

    // The ball itself moves toward the look.
    const centroid = (list: { x: number }[]) => list.reduce((sum, d) => sum + d.x, 0) / list.length;
    assert.ok(centroid(turned) > centroid(dots) + 3);
  });
});
